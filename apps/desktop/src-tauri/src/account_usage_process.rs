//! Small process readers with fixed bounds. stderr is discarded because it may
//! include command or provider data; neither argv nor logs contain a token.
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub(super) enum ProcessFailure {
    Spawn,
    Timeout,
    OutputTooLarge,
    Exit,
}

pub(super) fn run_to_exit(
    command: &mut Command,
    input: &[u8],
    timeout: Duration,
    max_output: usize,
    group: bool,
) -> Result<Vec<u8>, ProcessFailure> {
    // Put every provider subprocess in its own group. If a bounded helper
    // times out, its curl child must end with it rather than outlive XTrace.
    #[cfg(unix)]
    if group {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ProcessFailure::Spawn)?;
    let stdout = child.stdout.take().ok_or(ProcessFailure::Spawn)?;
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let mut stdout = stdout;
        let mut chunk = [0u8; 4096];
        let mut oversized = false;
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => {
                    if output.len().saturating_add(count) > max_output {
                        oversized = true;
                        break;
                    }
                    output.extend_from_slice(&chunk[..count]);
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(if oversized {
            Err(ProcessFailure::OutputTooLarge)
        } else {
            Ok(output)
        });
    });
    if let Some(mut stdin) = child.stdin.take()
        && stdin.write_all(input).is_err()
    {
        kill_tree(&mut child, group);
        let _ = child.wait();
        let _ = reader.join();
        return Err(ProcessFailure::Exit);
    }
    let deadline = Instant::now() + timeout;
    let mut status = None;
    let mut early_output = None;
    while Instant::now() < deadline {
        if let Ok(result) = rx.try_recv() {
            if result.is_err() {
                kill_tree(&mut child, group);
                let _ = child.wait();
                let _ = reader.join();
                return result;
            }
            early_output = Some(result);
        }
        match child.try_wait() {
            Ok(Some(exit)) => {
                status = Some(exit);
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break,
        }
    }
    if status.is_none() {
        kill_tree(&mut child, group);
        let _ = child.wait();
        let _ = reader.join();
        return Err(ProcessFailure::Timeout);
    }
    let output = match early_output {
        Some(result) => result?,
        None => rx
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| ProcessFailure::Exit)??,
    };
    let _ = reader.join();
    if !status.unwrap().success() && output.is_empty() {
        return Err(ProcessFailure::Exit);
    }
    Ok(output)
}

fn kill_tree(child: &mut std::process::Child, group: bool) {
    #[cfg(unix)]
    if group {
        unsafe {
            unsafe extern "C" {
                fn kill(pid: i32, signal: i32) -> i32;
            }
            if let Ok(pid) = i32::try_from(child.id()) {
                let _ = kill(-pid, 9);
            }
        }
    }
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_reader_accepts_small_output_and_rejects_large_output() {
        let mut echo = Command::new("/bin/cat");
        let output = run_to_exit(&mut echo, b"small", Duration::from_secs(1), 16, true).unwrap();
        assert_eq!(output, b"small");

        let mut echo = Command::new("/bin/cat");
        let failure = run_to_exit(
            &mut echo,
            b"larger than cap",
            Duration::from_secs(1),
            4,
            true,
        );
        assert!(matches!(failure, Err(ProcessFailure::OutputTooLarge)));
    }
}
