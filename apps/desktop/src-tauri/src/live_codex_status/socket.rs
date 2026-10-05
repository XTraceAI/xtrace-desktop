use super::{
    MAX_TARGETS, Shared, TICK,
    projection::Scanner,
    protocol::{self, Message, ThreadStatus},
};
use crate::dto::LiveSessionState;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt},
        net::UnixStream,
    },
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const BACKOFF: Duration = Duration::from_secs(1);
const RESYNC_INTERVAL: Duration = Duration::from_secs(2);
const MAX_RESYNCS: u8 = 3;

unsafe extern "C" {
    fn geteuid() -> u32;
}

/// Source location is native-only. Refuse links in every path component,
/// writable/shared IPC directories, and a socket owned by another account.
fn checked_socket(home: &Path) -> io::Result<PathBuf> {
    let path = home.join(".codex/ipc/ipc.sock");
    if !path.is_absolute() {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let mut walked = PathBuf::new();
    for component in path.components() {
        if !matches!(component, Component::RootDir | Component::Normal(_)) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        walked.push(component);
        if std::fs::symlink_metadata(&walked)?.file_type().is_symlink() {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
    }
    // SAFETY: geteuid has no arguments or memory effects.
    let uid = unsafe { geteuid() };
    let directory = std::fs::symlink_metadata(home.join(".codex/ipc"))?;
    let codex = std::fs::symlink_metadata(home.join(".codex"))?;
    let socket = std::fs::symlink_metadata(&path)?;
    if !directory.is_dir()
        || directory.uid() != uid
        || directory.mode() & 0o077 != 0
        || !codex.is_dir()
        || codex.uid() != uid
        || codex.mode() & 0o022 != 0
        || !socket.file_type().is_socket()
        || socket.uid() != uid
    {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(path)
}

/// A nonblocking Unix connect, bounded even when the listener's backlog is
/// full. Uses platform C ABI without introducing a dependency. Other Unix
/// platforms fail closed until their sockaddr ABI is supported.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn connect(path: &Path) -> io::Result<UnixStream> {
    use std::ffi::{c_int, c_short, c_void};
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    };
    #[repr(C)]
    struct Address {
        #[cfg(target_os = "macos")]
        len: u8,
        #[cfg(target_os = "macos")]
        family: u8,
        #[cfg(target_os = "linux")]
        family: u16,
        path: [u8; 108],
    }
    #[repr(C)]
    struct PollFd {
        fd: c_int,
        events: c_short,
        revents: c_short,
    }
    unsafe extern "C" {
        fn socket(domain: c_int, kind: c_int, protocol: c_int) -> c_int;
        #[link_name = "connect"]
        fn c_connect(fd: c_int, address: *const c_void, length: u32) -> c_int;
        #[cfg(target_os = "macos")]
        fn poll(fds: *mut PollFd, count: u32, timeout: c_int) -> c_int;
        #[cfg(target_os = "linux")]
        fn poll(fds: *mut PollFd, count: usize, timeout: c_int) -> c_int;
        fn fcntl(fd: c_int, command: c_int, argument: c_int) -> c_int;
    }
    let bytes = path.as_os_str().as_bytes();
    #[cfg(target_os = "macos")]
    let limit = 104;
    #[cfg(target_os = "linux")]
    let limit = 108;
    if bytes.contains(&0) || bytes.len() >= limit {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    // SAFETY: socket returns an owned descriptor or -1; from_raw_fd takes
    // ownership exactly once and closes it on all subsequent error paths.
    let fd = unsafe { socket(1, 1, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    // SAFETY: F_SETFD=2, FD_CLOEXEC=1 on these two supported platforms.
    if unsafe { fcntl(fd, 2, 1) } < 0 {
        return Err(io::Error::last_os_error());
    }
    stream.set_nonblocking(true)?;
    let mut address = Address {
        #[cfg(target_os = "macos")]
        len: (bytes.len() + 3) as u8,
        family: 1,
        path: [0; 108],
    };
    address.path[..bytes.len()].copy_from_slice(bytes);
    // SAFETY: address has the platform sockaddr_un prefix and a terminated
    // path; its checked length is within the allocated Address.
    let result = unsafe {
        c_connect(
            stream.as_raw_fd(),
            (&address as *const Address).cast(),
            (bytes.len() + 3) as u32,
        )
    };
    if result != 0 {
        let error = io::Error::last_os_error();
        #[cfg(target_os = "macos")]
        let in_progress = 36;
        #[cfg(target_os = "linux")]
        let in_progress = 115;
        if error.raw_os_error() != Some(in_progress) {
            return Err(error);
        }
        let mut fd = PollFd {
            fd: stream.as_raw_fd(),
            events: 4,
            revents: 0,
        };
        // SAFETY: poll receives one valid PollFd for at most 100ms.
        if unsafe { poll(&mut fd, 1, 100) } <= 0 {
            return Err(io::ErrorKind::TimedOut.into());
        }
        if let Some(error) = stream.take_error()? {
            return Err(error);
        }
        stream.peer_addr()?;
    }
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_millis(50)))?;
    stream.set_write_timeout(Some(Duration::from_millis(50)))?;
    Ok(stream)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn connect(_: &Path) -> io::Result<UnixStream> {
    Err(io::ErrorKind::Unsupported.into())
}

/// Body bytes read per `read` call, and the socket read size.
pub(super) const CHUNK: usize = 64 * 1024;
/// Per-call drain bound, so leases, releases and shutdown are still serviced
/// between reads while a large frame is arriving quickly.
const DRAIN_BYTES: usize = 4 * 1024 * 1024;
const DRAIN_TIME: Duration = Duration::from_millis(25);
/// A frame in progress fails when no byte arrives for REQUEST_TIMEOUT, or
/// when it has not finished after this long even while still trickling in.
const FRAME_DEADLINE: Duration = Duration::from_secs(60);

pub(super) enum Frame {
    Message(Box<Message>),
    /// A well-framed body that was malformed, over the projection bound or
    /// not a valid message. Carries its conversation when that was read.
    Skipped(Option<String>),
}

/// Streaming header/body reader. The declared length is validated first; the
/// body is then passed through the status projection in 64KiB reads and is
/// never held, so memory stays bounded for any frame size.
pub(super) struct Frames {
    header: [u8; 4],
    header_read: usize,
    remaining: usize,
    scanner: Option<Scanner>,
    skipped: Option<Option<String>>,
    buffer: Box<[u8]>,
    started: Option<Instant>,
    progressed: Option<Instant>,
}

impl Default for Frames {
    fn default() -> Self {
        Self {
            header: [0; 4],
            header_read: 0,
            remaining: 0,
            scanner: None,
            skipped: None,
            buffer: vec![0; CHUNK].into_boxed_slice(),
            started: None,
            progressed: None,
        }
    }
}

fn read_some(stream: &mut impl Read, buffer: &mut [u8]) -> io::Result<Option<usize>> {
    match stream.read(buffer) {
        Ok(0) => Err(io::ErrorKind::UnexpectedEof.into()),
        Ok(n) => Ok(Some(n)),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

impl Frames {
    pub fn in_frame(&self) -> bool {
        self.started.is_some()
    }

    /// Bytes retained for the frame in progress, for bound assertions.
    #[cfg(test)]
    pub fn retained(&self) -> usize {
        self.buffer.len() + self.scanner.as_ref().map_or(0, Scanner::retained)
    }

    /// Drains available bytes until one frame completes, the socket has no
    /// data, or this call's drain bound is reached.
    pub fn read(&mut self, stream: &mut impl Read) -> io::Result<Option<Frame>> {
        let call = Instant::now();
        let mut drained = 0;
        loop {
            if self
                .started
                .is_some_and(|at| at.elapsed() >= FRAME_DEADLINE)
                || self
                    .progressed
                    .is_some_and(|at| at.elapsed() >= REQUEST_TIMEOUT)
            {
                return Err(io::ErrorKind::TimedOut.into());
            }
            if drained >= DRAIN_BYTES || call.elapsed() >= DRAIN_TIME {
                return Ok(None);
            }
            let header = self.header_read < 4;
            let target = if header {
                &mut self.header[self.header_read..]
            } else {
                &mut self.buffer[..self.remaining.min(CHUNK)]
            };
            let Some(read) = read_some(stream, target)? else {
                return Ok(None);
            };
            let now = Instant::now();
            self.started.get_or_insert(now);
            self.progressed = Some(now);
            drained += read;
            if header {
                self.header_read += read;
                if self.header_read == 4 {
                    let length = u32::from_le_bytes(self.header) as usize;
                    if length == 0 || length > protocol::MAX_FRAME {
                        return Err(io::ErrorKind::InvalidData.into());
                    }
                    self.remaining = length;
                    self.scanner = Some(Scanner::default());
                }
                continue;
            }
            self.remaining -= read;
            if let Some(scanner) = &mut self.scanner
                && scanner.feed(&self.buffer[..read]).is_err()
            {
                // Keep framing: discard the rest of this body only.
                self.skipped = Some(scanner.conversation().map(str::to_owned));
                self.scanner = None;
            }
            if self.remaining > 0 {
                continue;
            }
            let frame = match self.scanner.take() {
                Some(scanner) => {
                    let conversation = scanner.conversation().map(str::to_owned);
                    match scanner
                        .finish()
                        .map_err(|_| ())
                        .and_then(|bytes| protocol::from_projection(&bytes))
                    {
                        Ok(message) => Frame::Message(Box::new(message)),
                        Err(()) => Frame::Skipped(conversation),
                    }
                }
                None => Frame::Skipped(self.skipped.take().flatten()),
            };
            let buffer = std::mem::take(&mut self.buffer);
            *self = Self {
                buffer,
                ..Self::default()
            };
            return Ok(Some(frame));
        }
    }
}

fn send(stream: &mut UnixStream, value: Value) -> io::Result<()> {
    let bytes = serde_json::to_vec(&value)?;
    if bytes.len() > 4096 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    // Small bounded outbound messages. Each write has a 50ms timeout.
    let mut frame = Vec::with_capacity(bytes.len() + 4);
    frame.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    frame.extend_from_slice(&bytes);
    write_bounded(stream, &frame)
}

fn write_bounded(stream: &mut UnixStream, mut bytes: &[u8]) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_millis(100);
    while !bytes.is_empty() {
        if Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        match stream.write(bytes) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn following(
    stream: &mut UnixStream,
    client: &str,
    native: &str,
    owner: &str,
    follow: bool,
) -> io::Result<()> {
    send(
        stream,
        json!({"type":"broadcast","sourceClientId":client,"version":1,
        "method":"thread-stream-following-changed","targetClientIds":[owner],
        "params":{"conversationId":native,"hostId":"local","following":follow}}),
    )
}

enum Request {
    Initialize,
    Discover { native: String, generation: u64 },
}
struct Pending {
    kind: Request,
    at: Instant,
}

struct Subscription {
    generation: u64,
    owner: Option<String>,
    // Keep the revision history even if this owner briefly disappears.
    revision_owner: Option<String>,
    status: ThreadStatus,
    attempted: Option<Instant>,
    followed: Option<Instant>,
    resyncs: u8,
}

impl Subscription {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            owner: None,
            revision_owner: None,
            status: ThreadStatus::default(),
            attempted: None,
            followed: None,
            resyncs: 0,
        }
    }
}

struct Connection {
    stream: UnixStream,
    frames: Frames,
    client: Option<String>,
    pending: BTreeMap<String, Pending>,
}

impl Connection {
    fn request(&mut self, kind: Request, sequence: &mut u64) -> io::Result<()> {
        if self.pending.len() >= MAX_TARGETS {
            return Ok(());
        }
        *sequence += 1;
        let request_id = format!("xtrace-status-{sequence}");
        let (method, params, version) = match &kind {
            Request::Initialize => (
                "initialize",
                json!({"clientType":"xtrace-status-observer"}),
                0,
            ),
            Request::Discover { native, .. } => (
                "thread-owner-discovery",
                json!({"hostId":"local","conversationId":native}),
                1,
            ),
        };
        send(
            &mut self.stream,
            json!({"type":"request","requestId":request_id,
            "sourceClientId":self.client.as_deref().unwrap_or(""),"version":version,
            "method":method,"params":params,"timeoutMs":2000}),
        )?;
        self.pending.insert(
            request_id,
            Pending {
                kind,
                at: Instant::now(),
            },
        );
        Ok(())
    }

    fn maintain(
        &mut self,
        subscriptions: &mut BTreeMap<String, Subscription>,
        sequence: &mut u64,
    ) -> io::Result<()> {
        let mut init_expired = false;
        self.pending.retain(|_, pending| {
            let keep = pending.at.elapsed() < REQUEST_TIMEOUT;
            if !keep && matches!(pending.kind, Request::Initialize) {
                init_expired = true;
            }
            keep
        });
        if init_expired {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let Some(client) = self.client.clone() else {
            return Ok(());
        };
        for (native, subscription) in subscriptions {
            if let Some(owner) = &subscription.owner {
                if subscription.status.needs_snapshot() && subscription.resyncs < MAX_RESYNCS
                    && subscription.followed.is_none_or(|at| at.elapsed() >= RESYNC_INTERVAL) {
                    following(&mut self.stream, &client, native, owner, true)?;
                    subscription.followed = Some(Instant::now());
                    subscription.resyncs += 1;
                }
            } else if self.pending.len() < MAX_TARGETS
                && subscription.attempted.is_none_or(|at| at.elapsed() >= REQUEST_TIMEOUT + BACKOFF)
                && !self.pending.values().any(|pending| matches!(&pending.kind, Request::Discover { native: id, .. } if id == native)) {
                self.request(Request::Discover { native: native.clone(), generation: subscription.generation }, sequence)?;
                subscription.attempted = Some(Instant::now());
            }
        }
        Ok(())
    }

    fn handle(
        &mut self,
        message: Message,
        subscriptions: &mut BTreeMap<String, Subscription>,
        shared: &Shared,
    ) -> io::Result<()> {
        match message.kind.as_str() {
            "client-discovery-request" => {
                if let Some(id) = message.request_id.filter(|id| protocol::client_id(id)) {
                    send(
                        &mut self.stream,
                        json!({"type":"client-discovery-response","requestId":id,"response":{"canHandle":false}}),
                    )?;
                }
            }
            "response" => {
                let Some(pending) = message
                    .request_id
                    .as_ref()
                    .and_then(|id| self.pending.remove(id))
                else {
                    return Ok(());
                };
                if pending.at.elapsed() >= REQUEST_TIMEOUT {
                    return if matches!(pending.kind, Request::Initialize) {
                        Err(io::ErrorKind::TimedOut.into())
                    } else {
                        Ok(())
                    };
                }
                let (expected_method, expected_version) = match &pending.kind {
                    Request::Initialize => ("initialize", 0),
                    Request::Discover { .. } => ("thread-owner-discovery", 1),
                };
                let valid = message.result_type.as_deref() == Some("success")
                    && message.method.as_deref() == Some(expected_method)
                    && message
                        .version
                        .is_none_or(|version| version == expected_version);
                match pending.kind {
                    Request::Initialize => {
                        if !valid {
                            return Err(io::ErrorKind::InvalidData.into());
                        }
                        self.client = message
                            .result
                            .and_then(|r| r.client_id)
                            .filter(|id| protocol::client_id(id));
                        if self.client.is_none() {
                            return Err(io::ErrorKind::InvalidData.into());
                        }
                    }
                    Request::Discover { native, generation } => {
                        // Serialize following=true with release. A response
                        // parsed while the view left cannot reopen its row.
                        let hub = shared
                            .hub
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if hub.closed
                            || hub
                                .targets
                                .get(&format!("codex-{native}"))
                                .is_none_or(|c| c.generation != generation)
                        {
                            return Ok(());
                        }
                        let Some(subscription) = subscriptions
                            .get_mut(&native)
                            .filter(|s| s.generation == generation)
                        else {
                            return Ok(());
                        };
                        if valid
                            && let Some(owner) = message.handled_by_client_id.filter(|id| {
                                protocol::client_id(id) && Some(id) != self.client.as_ref()
                            })
                        {
                            following(
                                &mut self.stream,
                                self.client.as_deref().ok_or(io::ErrorKind::InvalidData)?,
                                &native,
                                &owner,
                                true,
                            )?;
                            if subscription.revision_owner.as_ref() != Some(&owner) {
                                subscription.status.reset();
                            } else {
                                subscription.status.clear();
                            }
                            subscription.revision_owner = Some(owner.clone());
                            subscription.owner = Some(owner);
                            subscription.followed = Some(Instant::now());
                            subscription.resyncs = 0;
                        }
                    }
                }
            }
            "broadcast" if message.method.as_deref() == Some("ipc-connection-reset") => {
                // Use the worker's disconnect path: discard pending requests,
                // clear claims and require initialize/discovery/a new snapshot.
                return Err(io::ErrorKind::ConnectionReset.into());
            }
            "broadcast" if message.method.as_deref() == Some("thread-stream-state-changed") => {
                let Some(params) = message.params else {
                    return Ok(());
                };
                let Some((native, subscription)) = params
                    .conversation_id
                    .as_ref()
                    .and_then(|id| subscriptions.get_key_value(id))
                else {
                    return Ok(());
                };
                let native = native.clone();
                let valid = subscription.owner.is_some()
                    && message.source_client_id == subscription.owner
                    && message.version == Some(11)
                    && params.host_id.as_deref() == Some("local");
                let subscription = subscriptions
                    .get_mut(&native)
                    .expect("subscription just resolved");
                if !valid {
                    subscription.status.clear();
                    return Ok(());
                }
                if let Some(change) = params.change {
                    if subscription.status.apply(&native, change) {
                        subscription.resyncs = 0;
                    }
                } else {
                    subscription.status.clear();
                }
            }
            "broadcast" if message.method.as_deref() == Some("client-status-changed") => {
                let Some(params) = message.params else {
                    return Ok(());
                };
                let connected = params.connected == Some(true)
                    || params
                        .status
                        .as_ref()
                        .is_some_and(|v| v.as_str() == Some("connected"));
                if !connected {
                    for (native, subscription) in subscriptions {
                        if subscription.owner.is_some() && subscription.owner == params.client_id {
                            if let (Some(client), Some(owner)) = (&self.client, &subscription.owner)
                            {
                                following(&mut self.stream, client, native, owner, false)?;
                            }
                            subscription.owner = None;
                            subscription.status.clear();
                            subscription.attempted = Some(Instant::now());
                        }
                    }
                }
            }
            "client-status-changed" => {
                // The router can also send its own top-level client event.
                // An unfamiliar event shape fails closed for all owners.
                if message.status.as_ref().and_then(Value::as_str) != Some("connected") {
                    for (native, subscription) in subscriptions {
                        if message.client_id.is_none() || subscription.owner == message.client_id {
                            if let (Some(client), Some(owner)) = (&self.client, &subscription.owner)
                            {
                                following(&mut self.stream, client, native, owner, false)?;
                            }
                            subscription.owner = None;
                            subscription.status.clear();
                            subscription.attempted = Some(Instant::now());
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn unsubscribe(&mut self, subscriptions: &BTreeMap<String, Subscription>) {
        if let Some(client) = &self.client {
            // One bounded outbound batch, then disconnect, including on error.
            let mut frames = Vec::new();
            for (native, subscription) in subscriptions {
                if let Some(owner) = &subscription.owner {
                    let value = json!({"type":"broadcast","sourceClientId":client,"version":1,
                        "method":"thread-stream-following-changed","targetClientIds":[owner],
                        "params":{"conversationId":native,"hostId":"local","following":false}});
                    if let Ok(bytes) = serde_json::to_vec(&value) {
                        frames.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                        frames.extend_from_slice(&bytes);
                    }
                }
            }
            let _ = write_bounded(&mut self.stream, &frames);
        }
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

pub(super) fn run(home: PathBuf, shared: Arc<Shared>) {
    let mut connection: Option<Connection> = None;
    let mut subscriptions: BTreeMap<String, Subscription> = BTreeMap::new();
    let mut retry_at = Instant::now();
    let mut sequence = 0;
    loop {
        let (closed, targets) = {
            let mut hub = shared
                .hub
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            hub.expire(Instant::now());
            (
                hub.closed,
                hub.targets
                    .iter()
                    .filter_map(|(id, cache)| {
                        super::canonical_native(id)
                            .map(|native| (native.to_owned(), cache.generation))
                    })
                    .collect::<BTreeMap<_, _>>(),
            )
        };
        if closed {
            if let Some(connection) = &mut connection {
                connection.unsubscribe(&subscriptions);
            }
            return;
        }
        let removed: Vec<_> = subscriptions
            .iter()
            .filter(|(id, s)| targets.get(*id) != Some(&s.generation))
            .map(|(id, _)| id.clone())
            .collect();
        let mut failed = false;
        for native in removed {
            if let Some(subscription) = subscriptions.remove(&native)
                && let Some(connection) = &mut connection
            {
                if let (Some(client), Some(owner)) = (&connection.client, subscription.owner) {
                    failed |=
                        following(&mut connection.stream, client, &native, &owner, false).is_err();
                }
                connection.pending.retain(|_, p| !matches!(&p.kind, Request::Discover { native: id, .. } if *id == native));
            }
        }
        for (native, generation) in targets {
            subscriptions
                .entry(native)
                .or_insert_with(|| Subscription::new(generation));
        }
        if subscriptions.is_empty() {
            if let Some(mut connection) = connection.take() {
                connection.unsubscribe(&subscriptions);
            }
        } else if connection.is_none() && Instant::now() >= retry_at {
            let opened = (|| {
                let path = checked_socket(&home)?;
                let before = std::fs::symlink_metadata(&path)?;
                let stream = connect(&path)?;
                let after_path = checked_socket(&home)?;
                let after = std::fs::symlink_metadata(after_path)?;
                if before.dev() != after.dev() || before.ino() != after.ino() {
                    return Err(io::ErrorKind::PermissionDenied.into());
                }
                let mut connection = Connection {
                    stream,
                    frames: Frames::default(),
                    client: None,
                    pending: BTreeMap::new(),
                };
                connection.request(Request::Initialize, &mut sequence)?;
                Ok::<_, io::Error>(connection)
            })();
            match opened {
                Ok(opened) => connection = Some(opened),
                Err(_) => retry_at = Instant::now() + BACKOFF,
            }
        }
        let mut progressed = false;
        if let Some(connection) = &mut connection {
            if !failed {
                failed = connection
                    .maintain(&mut subscriptions, &mut sequence)
                    .is_err();
            }
            if !failed {
                match connection.frames.read(&mut connection.stream) {
                    Ok(Some(Frame::Message(message))) => {
                        progressed = true;
                        failed = connection
                            .handle(*message, &mut subscriptions, &shared)
                            .is_err()
                    }
                    Ok(Some(Frame::Skipped(Some(native)))) => {
                        // Framing is intact; only this conversation's status
                        // is unknown until a bounded resync delivers a snapshot.
                        progressed = true;
                        if let Some(subscription) = subscriptions.get_mut(&native) {
                            subscription.status.clear();
                        }
                    }
                    // An unattributable lost message may have been a reset or
                    // an owner change, so every claim is dropped instead.
                    Ok(Some(Frame::Skipped(None))) | Err(_) => failed = true,
                    Ok(None) => {}
                }
            }
        }
        if failed {
            if let Some(mut connection) = connection.take() {
                connection.unsubscribe(&subscriptions);
            }
            for subscription in subscriptions.values_mut() {
                subscription.owner = None;
                subscription.revision_owner = None;
                subscription.status.reset();
                subscription.attempted = None;
                subscription.followed = None;
                subscription.resyncs = 0;
            }
            retry_at = Instant::now() + BACKOFF;
        }
        let mut hub = shared
            .hub
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (native, subscription) in &subscriptions {
            if let Some(cache) = hub
                .targets
                .get_mut(&format!("codex-{native}"))
                .filter(|c| c.generation == subscription.generation)
            {
                cache.status = if connection.is_some() {
                    subscription.status.status()
                } else {
                    LiveSessionState::Unknown
                };
            }
        }
        if !hub.closed {
            // Do not delay large streaming frames or queued messages; each
            // iteration still checks rows, leases and shutdown between reads.
            if !progressed && connection.as_ref().is_none_or(|c| !c.frames.in_frame()) {
                let _ = shared.wake.wait_timeout(hub, TICK);
            }
        }
    }
}

#[cfg(test)]
mod response_tests {
    use super::super::Cached;
    use super::*;

    const NATIVE: &str = "11111111-1111-4111-8111-111111111111";

    fn pending_connection(
        initialize: bool,
        at: Instant,
    ) -> (
        Connection,
        BTreeMap<String, Subscription>,
        Shared,
        UnixStream,
    ) {
        let (stream, peer) = UnixStream::pair().unwrap();
        let shared = Shared::default();
        let mut subscriptions = BTreeMap::new();
        let kind = if initialize {
            Request::Initialize
        } else {
            shared.hub.lock().unwrap().targets.insert(
                format!("codex-{NATIVE}"),
                Cached {
                    generation: 1,
                    status: LiveSessionState::Unknown,
                },
            );
            subscriptions.insert(NATIVE.into(), Subscription::new(1));
            Request::Discover {
                native: NATIVE.into(),
                generation: 1,
            }
        };
        let connection = Connection {
            stream,
            frames: Frames::default(),
            client: (!initialize).then(|| "observer".into()),
            pending: [("request-1".into(), Pending { kind, at })].into(),
        };
        (connection, subscriptions, shared, peer)
    }

    fn response(initialize: bool) -> Value {
        json!({"type":"response","requestId":"request-1","resultType":"success",
            "method":if initialize { "initialize" } else { "thread-owner-discovery" },
            "handledByClientId":"owner","result":{"clientId":"observer"}})
    }

    fn parsed(value: Value) -> Message {
        protocol::parse(&serde_json::to_vec(&value).unwrap()).unwrap()
    }

    #[test]
    fn live_response_missing_or_wrong_methods_versions_and_failure_cannot_initialize_or_follow() {
        for initialize in [true, false] {
            let expected_version = if initialize { 0 } else { 1 };
            let mut invalid = Vec::new();
            let mut missing = response(initialize);
            missing.as_object_mut().unwrap().remove("method");
            invalid.push(missing);
            for method in [
                Value::Null,
                json!("other-method"),
                json!(if initialize {
                    "thread-owner-discovery"
                } else {
                    "initialize"
                }),
            ] {
                let mut value = response(initialize);
                value["method"] = method;
                invalid.push(value);
            }
            for version in [0, 1, 2, 11]
                .into_iter()
                .filter(|version| *version != expected_version)
            {
                let mut value = response(initialize);
                value["version"] = json!(version);
                invalid.push(value);
            }
            let mut failed = response(initialize);
            failed["resultType"] = json!("error");
            invalid.push(failed);
            for value in invalid {
                let (mut connection, mut subscriptions, shared, _peer) =
                    pending_connection(initialize, Instant::now());
                let result = connection.handle(parsed(value), &mut subscriptions, &shared);
                if initialize {
                    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
                    assert!(connection.client.is_none());
                } else {
                    result.unwrap();
                    assert!(subscriptions[NATIVE].owner.is_none());
                    assert_eq!(
                        subscriptions[NATIVE].status.status(),
                        LiveSessionState::Unknown
                    );
                }
                assert!(connection.pending.is_empty());
            }
        }
    }

    #[test]
    fn live_unknown_response_ids_leave_pending_requests_and_claims_untouched() {
        for initialize in [true, false] {
            let (mut connection, mut subscriptions, shared, _peer) =
                pending_connection(initialize, Instant::now());
            for id in [Value::Null, json!("unknown-request")] {
                let mut value = response(initialize);
                value["requestId"] = id;
                connection
                    .handle(parsed(value), &mut subscriptions, &shared)
                    .unwrap();
                assert_eq!(connection.pending.len(), 1);
                if initialize {
                    assert!(connection.client.is_none());
                } else {
                    assert!(subscriptions[NATIVE].owner.is_none());
                }
            }
        }
    }

    #[test]
    fn live_initializer_expiring_during_frame_read_returns_timeout_for_reconnect() {
        // A reply can finish parsing after maintain checked its deadline.
        let (mut connection, mut subscriptions, shared, _peer) =
            pending_connection(true, Instant::now() - REQUEST_TIMEOUT);
        let error = connection
            .handle(parsed(response(true)), &mut subscriptions, &shared)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(connection.client.is_none());
        assert!(connection.pending.is_empty());
    }

    #[test]
    fn live_initializer_expiring_without_reply_returns_timeout_for_reconnect() {
        let (mut connection, mut subscriptions, _shared, _peer) =
            pending_connection(true, Instant::now() - REQUEST_TIMEOUT);
        assert_eq!(
            connection
                .maintain(&mut subscriptions, &mut 0)
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
        assert!(connection.client.is_none());
        assert!(connection.pending.is_empty());
    }

    #[test]
    fn live_expired_discovery_reply_cannot_assign_an_owner() {
        let (mut connection, mut subscriptions, shared, _peer) =
            pending_connection(false, Instant::now() - REQUEST_TIMEOUT);
        connection
            .handle(parsed(response(false)), &mut subscriptions, &shared)
            .unwrap();
        assert!(subscriptions[NATIVE].owner.is_none());
        assert_eq!(
            subscriptions[NATIVE].status.status(),
            LiveSessionState::Unknown
        );
        assert!(connection.pending.is_empty());
    }
}
