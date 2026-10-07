//! Read-only macOS Claude registry adapter. No keys, argv, history, hooks or
//! subprocesses. Only a complete bounded census can make a live claim.
use super::canonical_claude;
use crate::dto::LiveSessionState;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{CString, c_char, c_int, c_void},
    fs::{File, Metadata, OpenOptions},
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd, IntoRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Component, Path},
    time::{Duration, Instant},
};

const MAX_ENTRIES: usize = 256;
const MAX_RECORDS: usize = 128;
const MAX_BYTES: usize = 8192;
const MAX_TOTAL: usize = 1024 * 1024;
const DEADLINE: Duration = Duration::from_millis(250);
// Darwin sys/fcntl.h. All opens are read-only, nonblocking and close-on-exec.
const NOFOLLOW: c_int = 0x100;
const FILE_FLAGS: c_int = 0x4 | NOFOLLOW | 0x0100_0000;
const DIRECTORY_FLAGS: c_int = FILE_FLAGS | 0x0010_0000;
const NOFOLLOW_ANY: c_int = 0x2000_0000;

unsafe extern "C" {
    fn geteuid() -> u32;
    fn openat(fd: c_int, path: *const c_char, flags: c_int, ...) -> c_int;
    #[cfg_attr(target_arch = "x86_64", link_name = "fdopendir$INODE64")]
    fn fdopendir(fd: c_int) -> *mut c_void;
    fn closedir(dir: *mut c_void) -> c_int;
    #[cfg_attr(target_arch = "x86_64", link_name = "readdir$INODE64")]
    fn readdir(dir: *mut c_void) -> *const Dirent;
    fn __error() -> *mut c_int;
}

// Darwin's 64-bit-inode dirent. Access only the returned record's declared
// name bytes, not the full trailing array (which may not be allocated).
#[repr(C)]
struct Dirent {
    ino: u64,
    seekoff: u64,
    reclen: u16,
    namlen: u16,
    kind: u8,
    name: [u8; 1024],
}

struct Directory(*mut c_void);
impl Directory {
    fn new(file: File) -> Option<Self> {
        let fd = file.into_raw_fd();
        // SAFETY: fd is owned and points to the already checked directory.
        let dir = unsafe { fdopendir(fd) };
        if dir.is_null() {
            // fdopendir takes ownership only on success.
            drop(unsafe { File::from_raw_fd(fd) });
            None
        } else {
            Some(Self(dir))
        }
    }

    fn next(&mut self) -> Result<Option<Vec<u8>>, ()> {
        // SAFETY: this DIR is exclusively owned until Drop. errno is local to
        // this thread. readdir's record remains valid until the next call.
        unsafe {
            *__error() = 0;
            let entry = readdir(self.0);
            if entry.is_null() {
                return if *__error() == 0 { Ok(None) } else { Err(()) };
            }
            let len = usize::from((*entry).namlen);
            let prefix = std::mem::offset_of!(Dirent, name);
            if len == 0 || len >= 1024 || usize::from((*entry).reclen) < prefix + len + 1 {
                return Err(());
            }
            let name = std::ptr::addr_of!((*entry).name).cast::<u8>();
            if *name.add(len) != 0 {
                return Err(());
            }
            Ok(Some(std::slice::from_raw_parts(name, len).to_vec()))
        }
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        // SAFETY: ownership transferred once by fdopendir; closes its fd too.
        unsafe {
            closedir(self.0);
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    session_id: String,
    pid: i32,
    pid_domain: String,
    proc_start: String,
    status: String,
    waiting_for: Option<String>,
    version: String,
    kind: String,
}

impl Record {
    fn parse(bytes: &[u8], pid: i32) -> Option<Self> {
        if bytes.len() > MAX_BYTES {
            return None;
        }
        let record: Self = serde_json::from_slice(bytes).ok()?;
        (record.pid == pid
            && pid > 0
            && canonical_claude(&record.session_id).is_some()
            && record.pid_domain == "darwin"
            && well_formed_version(&record.version)
            && matches!(record.kind.as_str(), "interactive" | "bg")
            && record.proc_start.len() == 24
            && matches!(record.status.as_str(), "busy" | "idle" | "waiting")
            && record
                .waiting_for
                .as_ref()
                .is_none_or(|reason| reason.len() <= 64))
        .then_some(record)
    }

    fn status(&self) -> LiveSessionState {
        match self.status.as_str() {
            "busy" => LiveSessionState::Running,
            "idle" => LiveSessionState::Idle,
            "waiting" => match self.waiting_for.as_deref() {
                Some("permission prompt" | "sandbox request") => LiveSessionState::WaitingApproval,
                Some("input needed" | "dialog open") => LiveSessionState::WaitingInput,
                _ => LiveSessionState::Unknown,
            },
            _ => LiveSessionState::Unknown,
        }
    }
}

/// Any dot-separated numeric version such as "2.1.288". Claude Code updates
/// often, so no exact version is pinned; the other field checks in
/// `Record::parse` and the later process start check catch a format change.
fn well_formed_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 32
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Birth {
    seconds: i64,
    micros: u32,
}
impl Birth {
    fn nanos(self) -> Option<i128> {
        (self.seconds > 0 && self.micros < 1_000_000)
            .then_some(i128::from(self.seconds) * 1_000_000_000 + i128::from(self.micros) * 1000)
    }
    fn english_utc(self) -> Option<String> {
        self.nanos()?;
        let time = jiff::Timestamp::from_second(self.seconds)
            .ok()?
            .to_zoned(jiff::tz::TimeZone::UTC);
        // Jiff's strftime uses English names independently of process locale.
        Some(time.strftime("%a %b %e %H:%M:%S %Y").to_string())
    }
}

// Exact Darwin sys/proc_info.h proc_bsdinfo ABI. Name fields are padding only:
// they are never examined or retained. No process arguments are requested.
#[repr(C)]
#[derive(Default)]
struct BsdInfo {
    flags: u32,
    status: u32,
    xstatus: u32,
    pid: u32,
    ppid: u32,
    uid: u32,
    gid: u32,
    ruid: u32,
    rgid: u32,
    svuid: u32,
    svgid: u32,
    reserved: u32,
    comm: [u8; 16],
    name: [u8; 32],
    nfiles: u32,
    pgid: u32,
    pjobc: u32,
    tdev: u32,
    tpgid: u32,
    nice: i32,
    start_seconds: u64,
    start_micros: u64,
}
const _: () = assert!(std::mem::size_of::<BsdInfo>() == 136);
#[link(name = "proc")]
unsafe extern "C" {
    fn proc_pidinfo(pid: c_int, flavor: c_int, arg: u64, buffer: *mut c_void, size: c_int)
    -> c_int;
}
fn process_birth(pid: i32, uid: u32) -> Option<Birth> {
    if pid <= 0 {
        return None;
    }
    let mut info = BsdInfo::default();
    // SAFETY: fixed-size initialized buffer, checked full return, Darwin-only.
    let size = std::mem::size_of::<BsdInfo>() as c_int;
    let read = unsafe { proc_pidinfo(pid, 3, 0, (&mut info as *mut BsdInfo).cast(), size) };
    if read != size
        || info.pid != pid as u32
        || info.uid != uid
        || info.ruid != uid
        || !matches!(info.status, 2 | 3)
    {
        return None;
    }
    let birth = Birth {
        seconds: i64::try_from(info.start_seconds).ok()?,
        micros: u32::try_from(info.start_micros).ok()?,
    };
    birth.nanos()?;
    Some(birth)
}

fn identity(meta: &Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
    (
        meta.dev(),
        meta.ino(),
        meta.len(),
        meta.mtime(),
        meta.mtime_nsec(),
        meta.ctime(),
        meta.ctime_nsec(),
    )
}
fn owned(meta: &Metadata, uid: u32, directory: bool) -> bool {
    meta.uid() == uid
        && meta.mode() & 0o022 == 0
        && if directory {
            meta.is_dir()
        } else {
            meta.is_file() && meta.nlink() == 1
        }
}
fn open_child(parent: &File, name: &str, directory: bool, uid: u32) -> Option<File> {
    let name = CString::new(name).ok()?;
    // SAFETY: checked literal directory names or decimal PID filename, no
    // slash, read-only flags and pinned parent descriptor; no mode argument.
    let fd = unsafe {
        openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            if directory {
                DIRECTORY_FLAGS
            } else {
                FILE_FLAGS
            },
        )
    };
    if fd < 0 {
        return None;
    }
    let file = unsafe { File::from_raw_fd(fd) };
    owned(&file.metadata().ok()?, uid, directory).then_some(file)
}
fn open_home(home: &Path, uid: u32) -> Option<File> {
    if !home.is_absolute()
        || home
            .components()
            .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
    {
        return None;
    }
    let file = OpenOptions::new()
        .read(true)
        // Darwin rejects combining O_NOFOLLOW and O_NOFOLLOW_ANY. The latter
        // checks every component, including the final one.
        .custom_flags((DIRECTORY_FLAGS & !NOFOLLOW) | NOFOLLOW_ANY)
        .open(home)
        .ok()?;
    owned(&file.metadata().ok()?, uid, true).then_some(file)
}
fn filename_pid(name: &[u8]) -> Option<i32> {
    let digits = name.strip_suffix(b".json")?;
    if digits.is_empty()
        || digits.len() > 10
        || digits[0] == b'0'
        || !digits.iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    std::str::from_utf8(digits)
        .ok()?
        .parse::<i32>()
        .ok()
        .filter(|pid| *pid > 0)
}

pub(super) fn scan(
    home: &Path,
    targets: &BTreeSet<String>,
    cancelled: &dyn Fn() -> bool,
) -> Option<BTreeMap<String, LiveSessionState>> {
    // SAFETY: geteuid has no arguments and cannot fail.
    scan_with(
        home,
        targets,
        cancelled,
        unsafe { geteuid() },
        process_birth,
        Instant::now(),
    )
}

fn scan_with(
    home: &Path,
    targets: &BTreeSet<String>,
    cancelled: &dyn Fn() -> bool,
    uid: u32,
    birth: impl Fn(i32, u32) -> Option<Birth>,
    started: Instant,
) -> Option<BTreeMap<String, LiveSessionState>> {
    let stopped = || started.elapsed() >= DEADLINE || cancelled();
    if stopped() {
        return None;
    }
    let home_dir = open_home(home, uid)?;
    let claude_dir = open_child(&home_dir, ".claude", true, uid)?;
    let sessions = open_child(&claude_dir, "sessions", true, uid)?;
    let before = sessions.metadata().ok()?;
    let mut directory = Directory::new(sessions.try_clone().ok()?)?;
    let mut entries = 0;
    let mut records = 0;
    let mut total = 0;
    let mut found = BTreeMap::new();
    let mut seen = BTreeSet::new();
    while let Some(name) = directory.next().ok()? {
        if stopped() {
            return None;
        }
        if name == b"." || name == b".." {
            continue;
        }
        entries += 1;
        if entries > MAX_ENTRIES {
            return None;
        }
        if !name.ends_with(b".json") {
            continue;
        }
        let pid = filename_pid(&name)?;
        records += 1;
        if records > MAX_RECORDS {
            return None;
        }
        let name = std::str::from_utf8(&name).ok()?;
        let mut file = open_child(&sessions, name, false, uid)?;
        let initial = file.metadata().ok()?;
        if initial.len() > MAX_BYTES as u64 {
            return None;
        }
        let first = birth(pid, uid);
        let mut bytes = Vec::new();
        (&mut file)
            .take((MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .ok()?;
        total += bytes.len();
        if bytes.len() > MAX_BYTES
            || bytes.len() as u64 != initial.len()
            || total > MAX_TOTAL
            || stopped()
        {
            return None;
        }
        let record = Record::parse(&bytes, pid)?;
        let final_meta = file.metadata().ok()?;
        let named = open_child(&sessions, name, false, uid)?.metadata().ok()?;
        if !owned(&final_meta, uid, false)
            || identity(&initial) != identity(&final_meta)
            || identity(&initial) != identity(&named)
        {
            return None;
        }
        // Keep at most 128 UUIDs transiently to detect duplicates anywhere in
        // the census. Only leased IDs and statuses are returned to the caller.
        if !seen.insert(record.session_id.clone()) {
            return None;
        }
        if !targets.contains(&record.session_id) {
            continue;
        }
        let first = first?;
        let mtime_ns =
            i128::from(initial.mtime()) * 1_000_000_000 + i128::from(initial.mtime_nsec());
        if first.micros == 0
            || !(1..1_000_000_000).contains(&initial.mtime_nsec())
            || mtime_ns < first.nanos()?
            || record.proc_start != first.english_utc()?
            || birth(pid, uid)? != first
        {
            return None;
        }
        let status = record.status();
        found.insert(record.session_id, status);
    }
    // Refuse a changing/incomplete directory, including replacement of either
    // named directory. All opens remain relative to verified descriptors.
    if stopped()
        || identity(&before) != identity(&sessions.metadata().ok()?)
        || !owned(&sessions.metadata().ok()?, uid, true)
        || identity(&sessions.metadata().ok()?)
            != identity(
                &open_child(&claude_dir, "sessions", true, uid)?
                    .metadata()
                    .ok()?,
            )
        || identity(&claude_dir.metadata().ok()?)
            != identity(
                &open_child(&home_dir, ".claude", true, uid)?
                    .metadata()
                    .ok()?,
            )
        || identity(&home_dir.metadata().ok()?) != identity(&open_home(home, uid)?.metadata().ok()?)
    {
        return None;
    }
    (!stopped()).then_some(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::{
        cell::Cell,
        fs::{FileTimes, Permissions},
        os::unix::fs::{PermissionsExt, symlink},
        time::UNIX_EPOCH,
    };

    const ID: &str = "11111111-1111-4111-8111-111111111111";
    const OTHER: &str = "22222222-2222-4222-8222-222222222222";
    const PID: i32 = 12345;

    struct Registry {
        root: tempfile::TempDir,
        birth: Birth,
    }
    impl Registry {
        fn new() -> Self {
            let root = tempfile::TempDir::new_in("/private/tmp").unwrap();
            std::fs::create_dir_all(root.path().join(".claude/sessions")).unwrap();
            let birth = Birth {
                seconds: jiff::Timestamp::now().as_second() - 3600,
                micros: 123456,
            };
            Self { root, birth }
        }
        fn value(&self, pid: i32) -> Value {
            json!({"sessionId":ID,"pid":pid,"pidDomain":"darwin","procStart":self.birth.english_utc().unwrap(),
                "status":"busy","version":"2.1.284","kind":"interactive"})
        }
        fn path(&self, name: &str) -> std::path::PathBuf {
            self.root.path().join(".claude/sessions").join(name)
        }
        fn write(&self, pid: i32, value: &Value) {
            std::fs::write(
                self.path(&format!("{pid}.json")),
                serde_json::to_vec(value).unwrap(),
            )
            .unwrap();
        }
        fn scan(&self) -> Option<BTreeMap<String, LiveSessionState>> {
            self.scan_birth(|_, _| Some(self.birth))
        }
        fn scan_birth(
            &self,
            birth: impl Fn(i32, u32) -> Option<Birth>,
        ) -> Option<BTreeMap<String, LiveSessionState>> {
            scan_with(
                self.root.path(),
                &[ID.to_owned()].into(),
                &|| false,
                unsafe { geteuid() },
                birth,
                Instant::now(),
            )
        }
    }

    #[test]
    fn claude_darwin_abi_matches_existing_test_only_libc() {
        assert_eq!(
            std::mem::size_of::<BsdInfo>(),
            std::mem::size_of::<libc::proc_bsdinfo>()
        );
        assert_eq!(
            std::mem::offset_of!(BsdInfo, start_seconds),
            std::mem::offset_of!(libc::proc_bsdinfo, pbi_start_tvsec)
        );
        assert_eq!(
            std::mem::offset_of!(BsdInfo, uid),
            std::mem::offset_of!(libc::proc_bsdinfo, pbi_uid)
        );
        assert_eq!(
            std::mem::offset_of!(Dirent, name),
            std::mem::offset_of!(libc::dirent, d_name)
        );
        assert_eq!(
            std::mem::offset_of!(Dirent, namlen),
            std::mem::offset_of!(libc::dirent, d_namlen)
        );
        assert_eq!(NOFOLLOW_ANY, libc::O_NOFOLLOW_ANY);
        assert_eq!(
            DIRECTORY_FLAGS,
            libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_DIRECTORY
        );
    }

    #[test]
    fn claude_known_versions_kinds_and_waiting_reasons_use_existing_states() {
        let registry = Registry::new();
        for version in ["2.1.280", "2.1.284", "2.1.288", "3.0.0"] {
            for kind in ["interactive", "bg"] {
                for (status, reason, expected) in [
                    ("busy", None, LiveSessionState::Running),
                    ("idle", None, LiveSessionState::Idle),
                    (
                        "waiting",
                        Some("permission prompt"),
                        LiveSessionState::WaitingApproval,
                    ),
                    (
                        "waiting",
                        Some("sandbox request"),
                        LiveSessionState::WaitingApproval,
                    ),
                    (
                        "waiting",
                        Some("input needed"),
                        LiveSessionState::WaitingInput,
                    ),
                    (
                        "waiting",
                        Some("dialog open"),
                        LiveSessionState::WaitingInput,
                    ),
                    ("waiting", Some("new reason"), LiveSessionState::Unknown),
                    ("waiting", None, LiveSessionState::Unknown),
                ] {
                    let mut value = registry.value(PID);
                    value["version"] = json!(version);
                    value["kind"] = json!(kind);
                    value["status"] = json!(status);
                    value["waitingFor"] = json!(reason);
                    registry.write(PID, &value);
                    assert_eq!(
                        registry.scan().unwrap()[ID],
                        expected,
                        "{version} {kind} {status} {reason:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn claude_bad_missing_duplicate_and_oversized_fields_fail_closed() {
        let registry = Registry::new();
        let good = registry.value(PID);
        for field in [
            "sessionId",
            "pid",
            "pidDomain",
            "procStart",
            "status",
            "version",
            "kind",
        ] {
            let mut value = good.clone();
            value.as_object_mut().unwrap().remove(field);
            assert!(
                Record::parse(&serde_json::to_vec(&value).unwrap(), PID).is_none(),
                "{field}"
            );
        }
        for (field, value) in [
            ("sessionId", json!("../escape")),
            ("sessionId", json!(ID.to_uppercase().replace('1', "A"))),
            ("pid", json!(PID + 1)),
            ("pid", json!(-1)),
            ("pid", json!(1.5)),
            ("pidDomain", json!("linux")),
            ("procStart", json!("local time")),
            ("version", json!("")),
            ("version", json!("2..1")),
            ("version", json!("2.1.x")),
            ("version", json!(".2.1")),
            ("version", json!("2.1.")),
            ("version", json!(" 2.1.288")),
            ("version", json!("2.1.288 ")),
            ("version", json!(format!("{}1", "1.".repeat(16)))),
            ("version", json!(2.1)),
            ("kind", json!("future")),
            ("status", json!("active")),
            ("waitingFor", json!(false)),
            ("waitingFor", json!("x".repeat(65))),
        ] {
            let mut candidate = good.clone();
            candidate[field] = value;
            assert!(
                Record::parse(&serde_json::to_vec(&candidate).unwrap(), PID).is_none(),
                "{field}"
            );
        }
        let bytes = serde_json::to_vec(&good).unwrap();
        let mut duplicate = bytes[..bytes.len() - 1].to_vec();
        duplicate.extend_from_slice(b",\"status\":\"idle\"}");
        assert!(Record::parse(&duplicate, PID).is_none());
        assert!(Record::parse(b"not JSON", PID).is_none());
        assert!(Record::parse(&vec![b' '; MAX_BYTES + 1], PID).is_none());
    }

    #[test]
    fn claude_version_must_be_well_formed_but_is_not_pinned() {
        let long = format!("{}1", "1.".repeat(15)); // 31 bytes
        let longest = format!("{}11", "1.".repeat(15)); // 32 bytes
        assert_eq!(longest.len(), 32);
        for version in ["2", "2.1.288", "3.0.0", "10.20.300", &long, &longest] {
            assert!(well_formed_version(version), "{version}");
        }
        for version in [
            "",
            "2..1",
            "2.1.x",
            ".2.1",
            "2.1.",
            " 2.1",
            "2.1 ",
            "2.1.288-beta",
            "+2.1",
        ] {
            assert!(!well_formed_version(version), "{version}");
        }
        assert!(!well_formed_version(&format!("{}1", "1.".repeat(16)))); // 33 bytes
    }

    #[test]
    fn claude_2_1_288_busy_record_is_running() {
        // Claude Code 2.1.288 adds bridgeSessionId and hostSessionId; an
        // exact version pin made this whole census fail, hiding every arc.
        let registry = Registry::new();
        let mut value = registry.value(PID);
        value["version"] = json!("2.1.288");
        value["bridgeSessionId"] = json!(null);
        value["hostSessionId"] = json!("local_00000000-0000-4000-8000-000000000000");
        registry.write(PID, &value);
        assert_eq!(registry.scan().unwrap()[ID], LiveSessionState::Running);
    }

    #[test]
    fn claude_private_extra_fields_and_key_contents_are_never_status_sources() {
        let registry = Registry::new();
        let mut value = registry.value(PID);
        value["statusUpdatedAt"] = json!(0); // age never changes a producer status
        value["logPath"] = json!("/does/not/exist/private-transcript");
        value["prompt"] = json!({"private":"DO NOT RETAIN"});
        registry.write(PID, &value);
        let key = registry.path(&format!("{PID}.key"));
        std::fs::write(&key, b"not JSON; DO NOT READ").unwrap();
        std::fs::set_permissions(&key, Permissions::from_mode(0o0)).unwrap();
        assert_eq!(registry.scan().unwrap()[ID], LiveSessionState::Running);
        assert_eq!(
            Record::parse(&serde_json::to_vec(&value).unwrap(), PID)
                .unwrap()
                .session_id,
            ID
        );
    }

    #[test]
    fn claude_english_utc_and_space_padded_day_are_fixed() {
        for (timestamp, expected) in [
            ("2026-10-01T05:37:03Z", "Thu Oct  1 05:37:03 2026"),
            ("2026-09-30T21:07:37Z", "Wed Sep 30 21:07:37 2026"),
            ("2026-03-08T10:00:00Z", "Sun Mar  8 10:00:00 2026"),
            ("2026-11-01T09:00:00Z", "Sun Nov  1 09:00:00 2026"),
        ] {
            let time: jiff::Timestamp = timestamp.parse().unwrap();
            assert_eq!(
                Birth {
                    seconds: time.as_second(),
                    micros: 123456
                }
                .english_utc()
                .unwrap(),
                expected
            );
        }
        assert!(
            Birth {
                seconds: -1,
                micros: 1
            }
            .english_utc()
            .is_none()
        );
        assert!(
            Birth {
                seconds: 1,
                micros: 1_000_000
            }
            .english_utc()
            .is_none()
        );
    }

    #[test]
    fn claude_dead_unverifiable_reused_or_changed_process_is_unknown() {
        let registry = Registry::new();
        registry.write(PID, &registry.value(PID));
        assert!(registry.scan_birth(|_, _| None).is_none());
        assert!(
            registry
                .scan_birth(|_, _| Some(Birth {
                    seconds: registry.birth.seconds + 1,
                    ..registry.birth
                }))
                .is_none()
        );
        let calls = Cell::new(0);
        assert!(
            registry
                .scan_birth(|_, _| {
                    calls.set(calls.get() + 1);
                    Some(Birth {
                        micros: registry.birth.micros + calls.get() - 1,
                        ..registry.birth
                    })
                })
                .is_none()
        );
        assert!(
            registry
                .scan_birth(|_, _| Some(Birth {
                    micros: 0,
                    ..registry.birth
                }))
                .is_none()
        );
    }

    #[test]
    fn claude_unmodified_stale_record_rejects_same_second_pid_reuse() {
        let registry = Registry::new();
        registry.write(PID, &registry.value(PID));
        let file = OpenOptions::new()
            .write(true)
            .open(registry.path(&format!("{PID}.json")))
            .unwrap();
        let nanos = registry.birth.nanos().unwrap() as u64;
        file.set_times(
            FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_nanos(nanos - 1000)),
        )
        .unwrap();
        assert!(registry.scan().is_none());
        file.set_times(
            FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_nanos(nanos + 1000)),
        )
        .unwrap();
        assert_eq!(registry.scan().unwrap()[ID], LiveSessionState::Running);
        file.set_times(
            FileTimes::new()
                .set_modified(UNIX_EPOCH + Duration::from_secs(registry.birth.seconds as u64)),
        )
        .unwrap();
        assert!(registry.scan().is_none());
    }

    #[test]
    fn claude_real_kernel_self_birth_matches_real_file_without_peer_access() {
        let registry = Registry::new();
        let uid = unsafe { geteuid() };
        let pid = std::process::id() as i32;
        let birth = process_birth(pid, uid).expect("macOS self process birth");
        assert!(process_birth(pid, uid.wrapping_add(1)).is_none());
        assert!(process_birth(0, uid).is_none());
        assert!(process_birth(i32::MAX, uid).is_none());
        let mut value = registry.value(pid);
        value["procStart"] = json!(birth.english_utc().unwrap());
        registry.write(pid, &value);
        assert_eq!(
            scan(registry.root.path(), &[ID.to_owned()].into(), &|| false).unwrap()[ID],
            LiveSessionState::Running
        );
        // This proves the kernel adapter, not a real Claude publisher.
    }

    #[test]
    fn claude_missing_relative_and_symlinked_paths_fail_closed() {
        let registry = Registry::new();
        registry.write(PID, &registry.value(PID));
        let uid = unsafe { geteuid() };
        assert!(open_home(Path::new("relative"), uid).is_none());
        assert!(open_home(&registry.root.path().join("absent"), uid).is_none());
        let alias = registry.root.path().join("alias");
        symlink(registry.root.path().join(".claude"), &alias).unwrap();
        assert!(open_home(&alias.join("sessions"), uid).is_none());
        let sessions = registry.root.path().join(".claude/sessions");
        std::fs::rename(&sessions, registry.root.path().join("saved-sessions")).unwrap();
        symlink(registry.root.path().join("saved-sessions"), &sessions).unwrap();
        assert!(registry.scan().is_none());
    }

    #[test]
    fn claude_record_links_nonregular_and_writable_files_fail_closed() {
        for kind in ["symlink", "hardlink", "directory", "fifo", "writable"] {
            let registry = Registry::new();
            let path = registry.path(&format!("{PID}.json"));
            match kind {
                "directory" => std::fs::create_dir(&path).unwrap(),
                "fifo" => {
                    let path = CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
                    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
                }
                _ => {
                    registry.write(PID, &registry.value(PID));
                    if kind == "writable" {
                        std::fs::set_permissions(&path, Permissions::from_mode(0o666)).unwrap();
                    } else {
                        let original = registry.root.path().join("original");
                        std::fs::rename(&path, &original).unwrap();
                        if kind == "symlink" {
                            symlink(&original, &path).unwrap();
                        } else {
                            std::fs::hard_link(&original, &path).unwrap();
                        }
                    }
                }
            }
            assert!(registry.scan().is_none(), "{kind}");
        }
    }

    #[test]
    fn claude_directory_permissions_and_other_uid_are_refused() {
        for relative in ["", ".claude", ".claude/sessions"] {
            let registry = Registry::new();
            registry.write(PID, &registry.value(PID));
            std::fs::set_permissions(
                registry.root.path().join(relative),
                Permissions::from_mode(0o777),
            )
            .unwrap();
            assert!(registry.scan().is_none(), "{relative}");
        }
        let registry = Registry::new();
        let meta = registry.root.path().metadata().unwrap();
        assert!(!owned(&meta, unsafe { geteuid() }.wrapping_add(1), true));
        assert!(
            scan_with(
                registry.root.path(),
                &[ID.to_owned()].into(),
                &|| false,
                unsafe { geteuid() }.wrapping_add(1),
                |_, _| Some(registry.birth),
                Instant::now()
            )
            .is_none()
        );
    }

    #[test]
    fn claude_changed_or_replaced_open_record_is_refused() {
        for replace in [false, true] {
            let registry = Registry::new();
            registry.write(PID, &registry.value(PID));
            let calls = Cell::new(0);
            assert!(
                registry
                    .scan_birth(|_, _| {
                        calls.set(calls.get() + 1);
                        if calls.get() == 1 {
                            if replace {
                                std::fs::rename(
                                    registry.path(&format!("{PID}.json")),
                                    registry.path("original"),
                                )
                                .unwrap();
                            }
                            let mut value = registry.value(PID);
                            value["status"] = json!("idle");
                            registry.write(PID, &value);
                        }
                        Some(registry.birth)
                    })
                    .is_none()
            );
        }
    }

    #[test]
    fn claude_duplicate_holders_and_incomplete_census_refuse_all_claims() {
        let registry = Registry::new();
        registry.write(PID, &registry.value(PID));
        registry.write(PID + 1, &registry.value(PID + 1));
        assert!(registry.scan().is_none());
        std::fs::remove_file(registry.path(&format!("{}.json", PID + 1))).unwrap();
        std::fs::write(registry.path("999.json"), b"broken").unwrap();
        assert!(registry.scan().is_none());
        std::fs::remove_file(registry.path("999.json")).unwrap();
        let mut other = registry.value(PID + 1);
        other["sessionId"] = json!(OTHER);
        registry.write(PID + 1, &other);
        assert_eq!(registry.scan().unwrap().len(), 1);
        other["pid"] = json!(PID + 2);
        registry.write(PID + 2, &other);
        assert!(registry.scan().is_none()); // duplicate even outside selection
    }

    #[test]
    fn claude_record_byte_entry_and_time_budgets_are_bounded() {
        let registry = Registry::new();
        registry.write(PID, &registry.value(PID));
        let path = registry.path(&format!("{PID}.json"));
        std::fs::write(&path, vec![b' '; MAX_BYTES + 1]).unwrap();
        assert!(registry.scan().is_none());
        registry.write(PID, &registry.value(PID));
        let uid = unsafe { geteuid() };
        assert!(
            scan_with(
                registry.root.path(),
                &[ID.to_owned()].into(),
                &|| false,
                uid,
                |_, _| Some(registry.birth),
                Instant::now() - DEADLINE
            )
            .is_none()
        );
        assert!(
            scan_with(
                registry.root.path(),
                &[ID.to_owned()].into(),
                &|| true,
                uid,
                |_, _| Some(registry.birth),
                Instant::now()
            )
            .is_none()
        );
        for n in 0..MAX_ENTRIES {
            std::fs::write(registry.path(&format!("{n}.key")), b"do not read").unwrap();
        }
        assert!(registry.scan().is_none());
        let records = Registry::new();
        for n in 1..=MAX_RECORDS + 1 {
            let mut value = records.value(n as i32);
            value["sessionId"] = json!(format!("{n:08x}-1111-4111-8111-111111111111"));
            records.write(n as i32, &value);
        }
        assert!(records.scan().is_none());
    }

    #[test]
    fn claude_filename_pid_must_be_exact_and_json_file_must_exist() {
        for name in [
            "0.json",
            "01.json",
            "-1.json",
            "2147483648.json",
            "1.json.key",
            "1.key",
            "bad.json",
        ] {
            assert!(filename_pid(name.as_bytes()).is_none(), "{name}");
        }
        assert_eq!(filename_pid(b"12345.json"), Some(PID));
        let registry = Registry::new();
        let uid = unsafe { geteuid() };
        let home = open_home(registry.root.path(), uid).expect("safe home descriptor");
        let claude = open_child(&home, ".claude", true, uid).expect("safe Claude descriptor");
        let sessions =
            open_child(&claude, "sessions", true, uid).expect("safe registry descriptor");
        let mut directory = Directory::new(sessions).expect("directory enumeration descriptor");
        assert!(directory.next().is_ok(), "Darwin directory entry layout");
        assert!(registry.scan().unwrap().is_empty());
        registry.write(PID, &registry.value(PID + 1));
        assert!(registry.scan().is_none());
    }

    #[test]
    fn claude_exact_byte_record_and_entry_limits_accept_a_complete_census() {
        let registry = Registry::new();
        for n in 1..=MAX_RECORDS {
            let mut value = registry.value(n as i32);
            value["sessionId"] = json!(format!("{n:08x}-1111-4111-8111-111111111111"));
            let mut bytes = serde_json::to_vec(&value).unwrap();
            bytes.resize(MAX_BYTES, b' ');
            std::fs::write(registry.path(&format!("{n}.json")), bytes).unwrap();
            std::fs::write(registry.path(&format!("{n}.key")), b"not read").unwrap();
        }
        assert_eq!(MAX_RECORDS * MAX_BYTES, MAX_TOTAL);
        assert_eq!(MAX_RECORDS * 2, MAX_ENTRIES);
        assert!(registry.scan().unwrap().is_empty());
        std::fs::write(registry.path("extra.key"), b"not read").unwrap();
        assert!(registry.scan().is_none());
    }

    #[test]
    fn claude_cancel_or_directory_change_during_read_discards_status() {
        for action in ["cancel", "add-entry", "replace-sessions", "replace-claude"] {
            let registry = Registry::new();
            registry.write(PID, &registry.value(PID));
            let changed = Cell::new(false);
            let result = scan_with(
                registry.root.path(),
                &[ID.to_owned()].into(),
                &|| action == "cancel" && changed.get(),
                unsafe { geteuid() },
                |_, _| {
                    if !changed.replace(true) {
                        match action {
                            "add-entry" => {
                                std::fs::write(registry.path("new.key"), b"not read").unwrap()
                            }
                            "replace-sessions" => {
                                let old = registry.root.path().join(".claude/sessions");
                                std::fs::rename(&old, registry.root.path().join(".claude/saved"))
                                    .unwrap();
                                std::fs::create_dir(&old).unwrap();
                            }
                            "replace-claude" => {
                                let old = registry.root.path().join(".claude");
                                std::fs::rename(&old, registry.root.path().join("saved")).unwrap();
                                std::fs::create_dir_all(old.join("sessions")).unwrap();
                            }
                            _ => {}
                        }
                    }
                    Some(registry.birth)
                },
                Instant::now(),
            );
            assert!(result.is_none(), "{action}");
        }
    }
}
