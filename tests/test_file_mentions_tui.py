#!/usr/bin/env python3
"""Exercise @ file selection in a real, isolated TUI without provider calls.

Usage: python3 tests/test_file_mentions_tui.py /absolute/path/to/jcode
Uses a private home, socket, repository, PTY and debug command files. Never
connects to the user's daemon or imports their credentials. Unix only.
"""

import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time


def wait_for(predicate, description, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.05)
    raise AssertionError(f"Timed out waiting for {description}")


def main():
    binary = str(Path(sys.argv[1]).resolve())
    with tempfile.TemporaryDirectory(
        prefix="at-tui-", dir=os.environ.get("JCODE_SCRATCH_DIR")
    ) as temporary:
        root = Path(temporary)
        repo = root / "repo"
        repo.mkdir()
        (repo / "src").mkdir()
        (repo / "docs").mkdir()
        (repo / "ignored").mkdir()
        (repo / "src/alpha.rs").write_text("fn alpha() {}\n")
        (repo / "src/beta.rs").write_text("fn beta() {}\n")
        (repo / "docs/space name.md").write_text("# Example\n")
        (repo / "ignored/hidden.rs").write_text("not a candidate\n")
        (repo / ".gitignore").write_text("ignored/\n")
        subprocess.run(["git", "init", "-q", str(repo)], check=True)
        runtime = root / "r"
        runtime.mkdir()
        socket = runtime / "s.sock"
        if len(str(socket).encode()) >= 100:
            raise AssertionError("Set JCODE_SCRATCH_DIR to a shorter private path for Unix sockets")
        command_path = root / "debug-command"
        response_path = root / "debug-response"
        home = root / "home"
        home.mkdir()
        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": str(home),
            "JCODE_HOME": str(home / "jcode"),
            "XDG_CONFIG_HOME": str(home / "config"),
            "JCODE_RUNTIME_DIR": str(runtime),
            "JCODE_THEME": "dark",
            "JCODE_OPENAI_COMPAT_API_BASE": "http://127.0.0.1:9/v1",
            "JCODE_OPENAI_COMPAT_DEFAULT_MODEL": "file-picker-test",
            "JCODE_OPENAI_COMPAT_LOCAL_ENABLED": "1",
            "JCODE_DEBUG_CONTROL": "1",
            "JCODE_CLIENT_SELFDEV_MODE": "1",
            "JCODE_DEBUG_CMD_PATH": str(command_path),
            "JCODE_DEBUG_RESPONSE_PATH": str(response_path),
            "DO_NOT_TRACK": "1",
            "TERM": "xterm-256color",
        }
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
        server = client = None
        drain_thread = None
        stop_drain = threading.Event()
        terminal_bytes = bytearray()
        capture_start = 0
        capture_query = None
        columns = 120

        def drain():
            while not stop_drain.is_set():
                try:
                    if not select.select([master], [], [], 0.1)[0]:
                        continue
                    chunk = os.read(master, 65536)
                except OSError:
                    break
                if not chunk:
                    break
                terminal_bytes.extend(chunk)

        def debug(command):
            if client.poll() is not None:
                raise AssertionError(f"TUI exited with {client.returncode}")
            response_path.unlink(missing_ok=True)
            pending = command_path.with_suffix(".pending")
            pending.write_text(command)
            pending.replace(command_path)
            wait_for(lambda: response_path.exists() and response_path.stat().st_size, command)
            response = response_path.read_text()
            response_path.unlink()
            return response

        def state():
            return json.loads(debug("state"))

        def press(key):
            os.write(master, {"enter": b"\r", "tab": b"\t", "esc": b"\x1b"}[key])
            time.sleep(0.15)

        def frame_contains(text):
            nonlocal capture_start, capture_query, columns
            if capture_query != text:
                capture_query = text
                capture_start = len(terminal_bytes)
                columns = 121 if columns == 120 else 120
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, columns, 0, 0))
                os.kill(client.pid, signal.SIGWINCH)
            # The debug frame records composer state, not late popover text.
            # A resize forces a complete real terminal render for text checks.
            output = bytes(terminal_bytes[capture_start:]).decode(errors="replace")
            output = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", output)
            return output if text in output else None

        def suggestions_visible():
            frame = debug("screen-json")
            if not frame.startswith("{"):
                return None
            return json.loads(frame)["state"]["has_suggestions"]

        try:
            with (root / "server.log").open("wb") as log:
                server = subprocess.Popen(
                    [binary, "--provider", "openai-compatible", "--no-update", "--socket", str(socket), "serve",
                     "--temporary-server", "--owner-pid", str(os.getpid()),
                     "--temp-idle-timeout-secs", "90"],
                    env=env, cwd=repo, stdout=log, stderr=log, start_new_session=True,
                )
            wait_for(lambda: socket.exists() or server.poll() is not None, "private daemon", timeout=90)
            assert server.poll() is None, (root / "server.log").read_text()
            with (root / "client.log").open("wb") as log:
                client = subprocess.Popen(
                    [binary, "--provider", "openai-compatible", "--no-update", "--socket", str(socket),
                     "--cwd", str(repo), "--debug-socket"],
                    env=env, cwd=repo, stdin=slave, stdout=slave, stderr=log,
                    start_new_session=True,
                )
            drain_thread = threading.Thread(target=drain, daemon=True)
            drain_thread.start()
            wait_for(lambda: frame_contains("Esc to skip onboarding"), "first-run onboarding")
            press("esc")
            debug("enable")
            debug("set_input:")
            os.write(master, b"Review @src/al")
            wait_for(lambda: state()["input"] == "Review @src/al", "typed file reference")
            wait_for(lambda: frame_contains("@src/alpha.rs"), "file suggestion in real frame")
            before = state()
            press("enter")
            selected = state()
            assert selected["input"].rstrip() == "Review @src/alpha.rs", selected
            assert selected["messages"] == before["messages"], selected
            assert not selected["processing"], selected
            print("PASS: @ picker renders; Enter inserts without submitting")

            debug("set_input:Compare @src/be")
            wait_for(lambda: frame_contains("@src/beta.rs"), "Tab candidate")
            press("tab")
            assert state()["input"].rstrip() == "Compare @src/beta.rs"
            print("PASS: Tab inserts the selected repository path")

            debug("set_input:Draft @src/al")
            wait_for(lambda: frame_contains("@src/alpha.rs"), "dismiss candidate")
            press("esc")
            assert state()["input"] == "Draft @src/al"
            wait_for(lambda: suggestions_visible() is False, "dismissed suggestion state")
            print("PASS: Esc dismisses suggestions without clearing the draft")

            debug("set_input:Email person@example.com")
            wait_for(lambda: suggestions_visible() is False, "email without file suggestions")
            debug("set_input:Check @ignored/")
            rendered = wait_for(lambda: frame_contains("No matching nonignored files"), "ignored file exclusion")
            assert "hidden.rs" not in rendered
            before = state()
            press("enter")
            after = state()
            assert after["input"] == before["input"], after
            assert after["messages"] == before["messages"], after
            print("PASS: email is not a mention; ignored files stay hidden; empty selection does not submit")

            debug("set_input:Read @docs/sp")
            wait_for(lambda: frame_contains("space name.md"), "spaced filename")
            press("enter")
            result = state()["input"]
            assert result.startswith("Read ") and "space name.md" in result, result
            assert '"' in result or "`" in result, result
            print("PASS: filenames with spaces are inserted as quoted references")
            print("PASS: isolated real-TUI file mention workflow")
        except Exception:
            for name in ["server.log", "client.log"]:
                path = root / name
                if path.exists():
                    print(f"--- {name} ---\n{path.read_text(errors='replace')[-8000:]}", file=sys.stderr)
            print(terminal_bytes[-3000:].decode(errors="replace"), file=sys.stderr)
            raise
        finally:
            # Only process groups launched above are signalled. No global stop command.
            for process in [client, server]:
                if process is not None and process.poll() is None:
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait(timeout=5)
            stop_drain.set()
            if drain_thread is not None:
                drain_thread.join(timeout=1)
            os.close(slave)
            os.close(master)


if __name__ == "__main__":
    main()
