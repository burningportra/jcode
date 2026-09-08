//! Spawning external clipboard helpers (`wl-copy`, `xclip`, `xsel`).
//!
//! Kept out of `helpers.rs` so the clipboard-ownership contract has one home
//! and its tests live next to it.

/// Pipe `text` into an external clipboard helper (`wl-copy`, `xclip`, `xsel`)
/// and report whether it took ownership of the selection.
///
/// These helpers fork and stay alive to serve paste requests, so the caller
/// must not block on `wait()`: that would hang for as long as the clipboard is
/// owned, and this runs on the UI thread from copy keybindings where a stall is
/// felt directly as input lag. Instead poll briefly for an early failure (e.g.
/// no display server) so the remaining fallbacks still run, then treat a live
/// child as success and reap it in the background. Stdin delivery uses a
/// nonblocking pipe and shares the early-exit deadline, so a helper that never
/// reads cannot stall the UI on a payload larger than the pipe buffer.
#[cfg(all(unix, any(test, not(target_os = "macos"))))]
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn copy_via_clipboard_helper(program: &str, args: &[&str], text: &str) -> bool {
    use std::io::{ErrorKind, Write};
    use std::os::fd::AsRawFd;
    use std::time::{Duration, Instant};

    let deadline = Instant::now() + Duration::from_millis(150);
    let Ok(mut child) = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return false;
    };

    let wrote = (|| {
        let Some(mut stdin) = child.stdin.take() else {
            return false;
        };
        let fd = stdin.as_raw_fd();
        // SAFETY: stdin owns this live pipe descriptor throughout delivery.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags == -1
            // SAFETY: preserve existing flags and only change this pipe's write end.
            || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1
        {
            return false;
        }

        let mut remaining = text.as_bytes();
        while !remaining.is_empty() {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            match stdin.write(remaining) {
                Ok(0) => return false,
                Ok(n) => remaining = &remaining[n..],
                Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    let mut pipe = libc::pollfd {
                        fd,
                        events: libc::POLLOUT,
                        revents: 0,
                    };
                    // Round down so poll never asks to wait beyond the deadline.
                    let timeout = deadline
                        .saturating_duration_since(Instant::now())
                        .as_millis()
                        .min(5) as libc::c_int;
                    // SAFETY: pipe is one initialized pollfd with a live descriptor.
                    let result = unsafe { libc::poll(&mut pipe, 1, timeout) };
                    if result < 0
                        && std::io::Error::last_os_error().kind() != ErrorKind::Interrupted
                    {
                        return false;
                    }
                    if pipe.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                        return false;
                    }
                }
                Err(_) => return false,
            }
        }
        true
    })(); // Close stdin before checking exit status, including on delivery failure.
    if !wrote {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    }

    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return true,
            // Exited nonzero (e.g. xclip with no DISPLAY): let the next
            // fallback try.
            Ok(Some(_)) => return false,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    // Still running: the helper became the selection owner,
                    // which is the success case.
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                    return true;
                }
                std::thread::sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(5)),
                );
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Tests for the external clipboard-helper spawn path (issue #684). They use
/// ordinary coreutils instead of real clipboard tools so they pass on headless
/// CI: what matters is the contract (writes stdin, does not block on a
/// long-lived owner, reports failure for a nonzero exit or a missing binary).
#[cfg(all(test, unix))]
mod tests {
    use super::copy_via_clipboard_helper;

    #[test]
    fn helper_that_exits_successfully_counts_as_a_copy() {
        assert!(copy_via_clipboard_helper("/bin/cat", &[], "hello"));
    }

    #[test]
    fn helper_that_exits_nonzero_falls_through() {
        assert!(!copy_via_clipboard_helper("/usr/bin/false", &[], "hello"));
    }

    #[test]
    fn missing_helper_binary_falls_through() {
        assert!(!copy_via_clipboard_helper(
            "jcode-nonexistent-clipboard-helper",
            &[],
            "hello"
        ));
    }

    #[test]
    fn large_payload_to_nonreading_helper_times_out() {
        let payload = "x".repeat(2 * 1024 * 1024);
        let start = std::time::Instant::now();
        // Finite lifetime also bounds the test if blocking writes regress.
        assert!(!copy_via_clipboard_helper("/bin/sleep", &["2"], &payload));
        assert!(
            start.elapsed() < std::time::Duration::from_secs(1),
            "stdin delivery blocked for {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn large_consuming_helper_receives_entire_payload() {
        let payload = "clipboard payload\n".repeat(32 * 1024);
        let dir = tempfile::tempdir().expect("output directory");
        let output = dir.path().join("payload");
        let complete = dir.path().join("complete");
        assert!(copy_via_clipboard_helper(
            "/bin/sh",
            &[
                "-c",
                "/bin/cat > \"$1\" && : > \"$2\"",
                "clipboard-test",
                output.to_str().unwrap(),
                complete.to_str().unwrap(),
            ],
            &payload,
        ));
        // Success can mean a live owner: wait for the reader to drain the pipe.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !complete.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "reader did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(std::fs::read_to_string(output).unwrap(), payload);
    }

    /// A helper that keeps running is the success case (it owns the selection),
    /// and must not block the UI thread for its whole lifetime.
    #[test]
    fn long_lived_helper_counts_as_a_copy_without_blocking() {
        let start = std::time::Instant::now();
        assert!(copy_via_clipboard_helper("/bin/sleep", &["2"], "hello"));
        assert!(
            start.elapsed() < std::time::Duration::from_secs(1),
            "spawn path blocked for {:?}",
            start.elapsed()
        );
    }
}

/// Runtime ordering tests for issue #684. `copy_to_clipboard` itself is
/// short-circuited under `cfg(test)` (it must never touch the developer's real
/// clipboard), so these exercise the same fallback chain it runs, using stub
/// scripts passed to /bin/sh. That pins the property that actually matters:
/// the first helper that takes ownership wins, and one that fails fast hands
/// off to the next instead of reporting a false success.
#[cfg(all(test, unix))]
mod ordering_tests {
    use super::copy_via_clipboard_helper;
    /// Same chain as the Linux arm of `copy_to_clipboard`, reporting which
    /// helper claimed the clipboard. Missing stubs use genuinely absent paths.
    fn first_helper_that_wins(stubs: &[(&str, &str)]) -> Option<&'static str> {
        let dir = tempfile::tempdir().expect("stub directory");
        for (name, body) in stubs {
            std::fs::write(dir.path().join(name), body).expect("write stub");
        }
        for (program, args) in [
            ("wl-copy", &[][..]),
            ("xclip", &["-selection", "clipboard"][..]),
            ("xsel", &["--clipboard", "--input"][..]),
        ] {
            let script = dir.path().join(program);
            let path = script.to_str().unwrap();
            let copied = if script.exists() {
                // Fresh executable scripts can take >150ms to launch on macOS.
                // Use the system shell without changing PATH or the product deadline.
                let mut shell_args = vec![path];
                shell_args.extend_from_slice(args);
                copy_via_clipboard_helper("/bin/sh", &shell_args, "payload")
            } else {
                copy_via_clipboard_helper(path, args, "payload")
            };
            if copied {
                return Some(program);
            }
        }
        None
    }

    /// Regression guard: adding the X11 helpers must not steal the Wayland path
    /// that already worked.
    #[test]
    fn wayland_still_wins_when_wl_copy_works() {
        let winner = first_helper_that_wins(&[
            ("wl-copy", "/bin/cat > /dev/null; exit 0"),
            ("xclip", "exit 0"),
        ]);
        assert_eq!(winner, Some("wl-copy"));
    }

    /// The actual #684 scenario: no Wayland display, so wl-copy fails fast and
    /// xclip must take over instead of falling through to arboard.
    #[test]
    fn xclip_takes_over_when_wl_copy_fails() {
        let winner = first_helper_that_wins(&[
            ("wl-copy", "exit 1"),
            ("xclip", "/bin/cat > /dev/null; exit 0"),
            ("xsel", "exit 0"),
        ]);
        assert_eq!(winner, Some("xclip"));
    }

    #[test]
    fn xsel_takes_over_when_wl_copy_and_xclip_are_missing() {
        let winner = first_helper_that_wins(&[("xsel", "/bin/cat > /dev/null; exit 0")]);
        assert_eq!(winner, Some("xsel"));
    }

    /// With no working helper the chain must report failure, so the real
    /// `copy_to_clipboard` continues to arboard and then OSC 52 rather than
    /// showing a false "Copied" toast.
    #[test]
    fn no_working_helper_reports_failure_so_later_fallbacks_run() {
        let winner = first_helper_that_wins(&[
            ("wl-copy", "exit 1"),
            ("xclip", "exit 1"),
            ("xsel", "exit 1"),
        ]);
        assert_eq!(winner, None);
    }
}
