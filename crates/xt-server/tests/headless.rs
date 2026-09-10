use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};
struct ChildServer(Child, u16);
impl Drop for ChildServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start(path: &std::path::Path, extra: &[&str]) -> ChildServer {
    let mut child = Command::new(env!("CARGO_BIN_EXE_xtrace-core"))
        .args(["serve", "--db"])
        .arg(path)
        .args(extra)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(line);
    });
    let mut owner = ChildServer(child, 0);
    let line = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("Headless server must report bound port");
    owner.1 = line
        .trim()
        .strip_prefix("XTRACE_PORT=")
        .unwrap()
        .parse()
        .unwrap();
    owner
}
fn fail(path: &std::path::Path, args: &[&str]) -> String {
    let child = Command::new(env!("CARGO_BIN_EXE_xtrace-core"))
        .args(["serve", "--db"])
        .arg(path)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut owner = ChildServer(child, 0);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = owner.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "Invalid startup must fail promptly"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(!status.success());
    let mut output = String::new();
    std::io::Read::read_to_string(owner.0.stdout.as_mut().unwrap(), &mut output).unwrap();
    assert!(output.is_empty());
    std::io::Read::read_to_string(owner.0.stderr.as_mut().unwrap(), &mut output).unwrap();
    output
}

#[test]
fn loopback_headless_binary_port_zero_conflict_and_invalid_bind_are_real() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("synthetic.db");
    let server = start(&path, &["--port", "0"]);
    assert_ne!(server.1, 0);
    assert!(
        fail(
            &dir.path().join("second.db"),
            &["--port", &server.1.to_string()]
        )
        .contains("occupied")
    );
    let rejected = dir.path().join("must-not-create.db");
    assert!(fail(&rejected, &["--bind", "0.0.0.0"]).contains("localhost"));
    assert!(!rejected.exists());
    assert!(fail(&rejected, &["--port", "-1"]).contains("Invalid"));
    assert!(fail(&rejected, &["--port", "0", "--port", "1"]).contains("duplicate"));
}
#[test]
fn loopback_headless_reads_persisted_port_and_explicit_override_wins() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("synthetic.db");
    drop(xt_store::Store::open(&path).unwrap());
    let sql = rusqlite::Connection::open(&path).unwrap();
    // Keep the selected configured port occupied: startup must read it and fail,
    // rather than silently falling back to the default or an ephemeral listener.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    sql.execute(
        "INSERT INTO settings VALUES ('server.port',?1)",
        [port.to_string()],
    )
    .unwrap();
    assert!(fail(&path, &[]).contains("occupied"));
    let override_server = start(&path, &["--port", "0"]);
    assert_ne!(override_server.1, port);
    drop(override_server);
    for value in ["0", "-1", "65536", "1.5", "null", "\"47421\"", "{}"] {
        sql.execute(
            "UPDATE settings SET value_json=?1 WHERE key='server.port'",
            [value],
        )
        .unwrap();
        assert!(fail(&path, &[]).contains("server.port"));
    }
    let override_server = start(&path, &["--port", "0"]);
    assert_ne!(override_server.1, 0);
}
