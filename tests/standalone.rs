use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn read_until(
    output: &mut pty_process::OwnedReadPty,
    seen: &mut Vec<u8>,
    ready: impl Fn(&[u8]) -> bool,
) {
    let mut bytes = [0; 8192];
    while !ready(seen) {
        let n = output.read(&mut bytes).await.expect("standalone output");
        assert_ne!(n, 0, "standalone ended before expected output");
        seen.extend_from_slice(&bytes[..n]);
        assert!(seen.len() < 1 << 20, "unexpected output flood");
    }
}

fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|part| part == needle)
}

async fn standalone_exit(signal: bool) {
    let (pty, pts) = pty_process::open().unwrap();
    pty.resize(pty_process::Size::new(30, 120)).unwrap();
    let key =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/nonexistent-standalone-key");
    assert!(!key.exists(), "test must never load a real device key");
    let mut child = pty_process::Command::new(env!("CARGO_BIN_EXE_dev-null-addict"))
        .env("NO_DEV_NULL_ADDICT", "1")
        .env("SHELL", "/standalone-must-not-run-a-shell")
        .env("TERM", "xterm-256color")
        .arg("--key")
        .arg(key)
        .kill_on_drop(true)
        .spawn(pts)
        .unwrap();
    let (mut output, mut input) = pty.into_split();
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut seen = Vec::new();
        read_until(&mut output, &mut seen, |bytes| {
            let mut screen = vt100::Parser::new(30.try_into().unwrap(), 120.try_into().unwrap(), 0);
            screen.process(bytes);
            let text = screen.screen().contents();
            text.contains("Not signed in.") && text.contains("back / quit")
        })
        .await;
        assert!(contains(&seen, b"\x1b[?1049h"));
        assert!(contains(&seen, b"\x1b[>0u"));
        assert!(contains(&seen, b"\x1b[?2004h"));
        assert!(
            !contains(&seen, b"\x1b[6n"),
            "standalone must not probe a screen copy"
        );
        if signal {
            assert!(
                tokio::process::Command::new("kill")
                    .args(["-TERM", &child.id().unwrap().to_string()])
                    .kill_on_drop(true)
                    .status()
                    .await
                    .unwrap()
                    .success()
            );
        } else {
            input.write_all(b"\x1b").await.unwrap();
        }
        read_until(&mut output, &mut seen, |bytes| {
            contains(bytes, b"\x1b[?1049l")
        })
        .await;
        assert!(contains(&seen, b"\x1b[<u"));
        assert!(contains(&seen, b"\x1b[?2004l"));
        assert!(contains(&seen, b"\x1b[?25h"));
        // Polling also works where the test runtime does not receive SIGCHLD.
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert_eq!(status.code(), Some(if signal { 143 } else { 0 }));
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("bounded standalone exit test");
}

// A missing key still draws the full UI inside a wrapper, and Esc restores the terminal and exits zero.
#[tokio::test(flavor = "current_thread")]
async fn standalone_escape_restores_terminal() {
    standalone_exit(false).await;
}

// SIGTERM must restore the standalone terminal before returning the conventional signal exit code.
#[tokio::test(flavor = "current_thread")]
async fn standalone_sigterm_restores_terminal() {
    standalone_exit(true).await;
}

// Standalone parse and non-TTY errors must retain their exit codes without attempting shell recovery.
#[tokio::test(flavor = "current_thread")]
async fn standalone_errors_do_not_fall_back() {
    tokio::time::timeout(Duration::from_secs(5), async {
        for (args, code, message) in [
            (
                vec![],
                1,
                "dev-null-addict: standard input and output have to be a terminal",
            ),
            (vec!["--unknown"], 2, "error:"),
            (vec!["--", "--toggle=ctrl+g"], 2, "error:"),
            (vec!["init", "sh"], 2, "error:"),
        ] {
            let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_dev-null-addict"))
                .env("NO_DEV_NULL_ADDICT", "1")
                .env("SHELL", "/standalone-must-not-run-a-shell")
                .args(args)
                .kill_on_drop(true)
                .output()
                .await
                .unwrap();
            assert_eq!(output.status.code(), Some(code));
            let error = String::from_utf8_lossy(&output.stderr);
            assert!(error.contains(message), "{error}");
            assert!(
                !error.contains("fallback") && !error.contains("startup files"),
                "{error}"
            );
            assert!(output.stdout.is_empty());
        }
    })
    .await
    .expect("bounded standalone CLI test");
}
