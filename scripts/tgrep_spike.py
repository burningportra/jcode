#!/usr/bin/env python3
"""Bounded tgrep v1.0.4 experiment. No installs or production modifications.

Only the parent should execute this spike. All subprocess output is temporary,
size-monitored, and excluded from the report. Parity failures are observations.
"""

import argparse
import base64
from collections import Counter
import json
import math
import os
from pathlib import Path
import signal
import socket
import statistics
import subprocess
import tempfile
import time


RG = "/opt/homebrew/bin/rg"
WARMUPS = 5
OUTPUT_LIMIT = 16 * 1024**2
INDEX_LIMIT = 2 * 1024**3
RSS_LIMIT = 1024**3
AT_FLAGS = ["--hidden", "--no-require-git", "-g", "!.git"]
SAFE_AT_FLAGS = ["--hidden", "--no-require-git", "-g", "!.git/**"]
POLICIES = {"default_indexed": [], "at_compatible": AT_FLAGS}
SENTINEL = "TGREP_SPIKE_SENTINEL_739184"


class SafetyError(RuntimeError):
    """Stop the entire experiment, but retain the partial report."""


class CommandError(RuntimeError):
    """A bounded command failed. Its metadata is already in the report."""


def within(path, root):
    return path == root or root in path.parents


def disk_bytes(root):
    total = 0
    for directory, _, files in os.walk(root, followlinks=False):
        for name in files:
            try:
                total += (Path(directory) / name).lstat().st_size
            except FileNotFoundError:
                pass
    return total


def stop_owned(process):
    # Only Popen objects created with start_new_session=True reach this function.
    # Do not infer ownership from serve.json, and never signal an arbitrary PID.
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=1)
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    process.wait(timeout=2)


class Runner:
    """One owner for process groups, temporary output, and monitored budgets."""

    def __init__(self, args, work, report):
        self.args, self.work, self.report = args, work, report
        self.deadline = time.monotonic() + 600
        self.next_monitor = 0
        self.processes = {}
        self.fixture_temps = []
        self.indexes = work / "indexes"
        self.indexes.mkdir()
        self.env = {"PATH": "/usr/bin:/bin:/opt/homebrew/bin", "LC_ALL": "C",
                    "HOME": str(work / "home"), "TMPDIR": str(work / "tmp"),
                    "XDG_CONFIG_HOME": str(work / "home" / "config"),
                    "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
                    "GIT_TERMINAL_PROMPT": "0",
                    "RAYON_NUM_THREADS": str(max(1, (os.cpu_count() or 1) // 4))}
        (work / "home").mkdir()
        (work / "tmp").mkdir()
        report["monitor"] = {"peak_index_bytes": 0, "peak_server_rss_bytes": 0,
                             "samples": [], "rss_unavailable": 0}

    def index(self, label):
        path = self.indexes / label
        path.mkdir()
        return path

    def argv(self, tool, index, flags):
        if tool == "tgrep":
            return [str(self.args.tgrep), "--index-path", str(index), *flags]
        return [RG, *flags]

    def start(self, argv, root, server=False):
        self.check()
        out = tempfile.TemporaryFile(dir=self.work / "tmp")
        err = tempfile.TemporaryFile(dir=self.work / "tmp")
        try:
            process = subprocess.Popen(argv, cwd=root, env=self.env, stdin=subprocess.DEVNULL,
                                       stdout=out, stderr=err, start_new_session=True)
        except BaseException:
            out.close()
            err.close()
            raise
        self.processes[process] = (out, err, server)
        return process

    def finish(self, process):
        out, err, _ = self.processes[process]
        try:
            stop_owned(process)
        finally:
            out.close()
            err.close()
            del self.processes[process]

    def check(self, force=False):
        now = time.monotonic()
        if now >= self.deadline:
            raise SafetyError("total_run_600s_exceeded")
        for process, (out, err, _) in self.processes.items():
            if os.fstat(out.fileno()).st_size + os.fstat(err.fileno()).st_size > OUTPUT_LIMIT:
                raise SafetyError("combined_command_output_16MiB_exceeded")
        if not force and now < self.next_monitor:
            return
        self.next_monitor = now + 0.5
        size = disk_bytes(self.indexes)
        monitor = self.report["monitor"]
        monitor["peak_index_bytes"] = max(size, monitor["peak_index_bytes"])
        if size > INDEX_LIMIT:
            raise SafetyError("combined_indexes_2GiB_exceeded")
        rss = 0
        for process, (_, _, server) in self.processes.items():
            if not server or process.poll() is not None:
                continue
            # ps has fixed, tiny output. Give even this helper an owned group.
            with tempfile.TemporaryFile(dir=self.work / "tmp") as output:
                helper = subprocess.Popen(["/bin/ps", "-o", "rss=", "-p", str(process.pid)],
                                          stdout=output, stderr=subprocess.DEVNULL,
                                          stdin=subprocess.DEVNULL, start_new_session=True,
                                          env=self.env)
                try:
                    helper.wait(timeout=1)
                    output.seek(0)
                    value = int(output.read(128).strip()) * 1024
                    rss += value
                except (ValueError, subprocess.TimeoutExpired):
                    monitor["rss_unavailable"] += 1
                finally:
                    stop_owned(helper)
        monitor["peak_server_rss_bytes"] = max(rss, monitor["peak_server_rss_bytes"])
        monitor["samples"].append({"elapsed_s": round(600 - (self.deadline - now), 3),
                                   "index_bytes": size, "server_rss_bytes": rss})
        if rss > RSS_LIMIT:
            raise SafetyError("server_RSS_1GiB_exceeded")

    def run(self, argv, root, timeout=30, allowed=(0, 1)):
        record = {"argv": list(map(str, argv)), "cwd": str(root)}
        self.report["commands"].append(record)
        process = None
        self.check()
        start = time.perf_counter()
        try:
            process = self.start(argv, root)
            end = time.monotonic() + timeout
            while process.poll() is None:
                self.check()
                if time.monotonic() >= end:
                    raise CommandError("command_timeout")
                time.sleep(0.001)
            record["latency_ms"] = (time.perf_counter() - start) * 1000
            self.check(force=True)
            out, err, _ = self.processes[process]
            record.update(returncode=process.returncode,
                          stdout_bytes=os.fstat(out.fileno()).st_size,
                          stderr_bytes=os.fstat(err.fileno()).st_size)
            if process.returncode not in allowed:
                raise CommandError("command_exit_" + str(process.returncode))
            out.seek(0)
            return out.read(OUTPUT_LIMIT), record["latency_ms"]
        except BaseException as exc:
            record["error"] = type(exc).__name__
            if isinstance(exc, (CommandError, SafetyError)):
                record["reason"] = str(exc)
            record["elapsed_ms"] = (time.perf_counter() - start) * 1000
            raise
        finally:
            if process is not None:
                self.finish(process)

    def close(self):
        for process in list(self.processes):
            try:
                self.finish(process)
            except Exception as exc:
                self.report["errors"].append({"phase": "cleanup", "type": type(exc).__name__})
        for directory in self.fixture_temps:
            try:
                directory.cleanup()
            except Exception as exc:
                self.report["errors"].append({"phase": "fixture_cleanup", "type": type(exc).__name__})


def field_bytes(value):
    if "text" in value:
        return value["text"].encode("utf-8")
    return base64.b64decode(value["bytes"], validate=True)


def normalize_path(raw, root):
    # Preserve .git, symlinks, control characters, and invalid UTF-8 here. Raw
    # parity must not silently apply the @ adapter's exclusions.
    path = os.fsdecode(raw)
    if os.path.isabs(path):
        return os.path.relpath(path, root)
    return path[2:] if path.startswith("./") else path


def matches(output, root):
    result = Counter()
    for line in output.splitlines():
        item = json.loads(line)
        if item.get("type") != "match":
            continue
        data = item["data"]
        path = normalize_path(field_bytes(data["path"]), root)
        # Keep full bytes in memory for exact parity, never in report artifacts.
        for sub in data["submatches"]:
            key = (path, data["line_number"], data.get("absolute_offset"),
                   field_bytes(data["lines"]), sub["start"], sub["end"],
                   field_bytes(sub["match"]))
            result[key] += 1
    return result


def listing(output, root):
    if output and not output.endswith(b"\0"):
        raise ValueError("non_NUL_terminated_listing")
    return {normalize_path(path, root) for path in output.split(b"\0") if path}


def adapter_paths(paths, root):
    result = set()
    for path in paths:
        parts = Path(path).parts
        if not parts or Path(path).is_absolute() or ".." in parts or ".git" in parts:
            continue
        if any(ord(char) < 32 or 127 <= ord(char) <= 159 or
               0xD800 <= ord(char) <= 0xDFFF for char in path):
            continue
        cursor = root
        for part in parts:
            cursor /= part
            if cursor.is_symlink():
                break
        else:
            result.add(path)
    return result


def difference(expected, actual, fixture=False):
    missing, extra = expected - actual, actual - expected
    count = lambda values: sum(values.values()) if isinstance(values, Counter) else len(values)
    result = {"equal": expected == actual, "expected_count": count(expected),
              "actual_count": count(actual), "missing_count": count(missing),
              "extra_count": count(extra)}
    if fixture and isinstance(expected, set):
        result.update(missing=sorted(missing), extra=sorted(extra))
    return result


def distribution(values):
    return {"raw_ms": values, "median_ms": statistics.median(values),
            "p95_ms": sorted(values)[math.ceil(len(values) * 0.95) - 1]}


def query(runner, root, index, tool, pattern, flags=(), timeout=30):
    data, latency = runner.run(runner.argv(tool, index, [*flags, "--json", "-e", pattern, "."]),
                               root, timeout=timeout)
    return matches(data, root), latency


def files(runner, root, index, tool, flags=(), timeout=30):
    data, latency = runner.run(runner.argv(tool, index, [*flags, "--files", "-0", "."]), root,
                               timeout=timeout)
    return listing(data, root), latency


def matching_files(runner, root, index, tool):
    data, latency = runner.run(runner.argv(tool, index, ["-F", "-l", "-0", "-e", "ToolOutput", "."]), root)
    return listing(data, root), latency


def phase(report, name, action):
    try:
        return action()
    except SafetyError:
        raise
    except Exception as exc:
        # Exception strings and stderr can contain repository source. Do not save them.
        report["errors"].append({"phase": name, "type": type(exc).__name__,
                                 "reason": str(exc) if isinstance(exc, CommandError) else "details withheld"})
        return None


def fixture(runner, root, git=True):
    root.mkdir()
    payloads = {"ordinary.txt": SENTINEL + "\nToolOutput alpha ALPHA\n",
                "untracked.txt": "ToolOutput alpha\n", "empty.txt": "",
                "binary.bin": b"\x00ToolOutput alpha\x00",
                ".hidden.txt": "ToolOutput alpha\n",
                "dir with spaces/Prefix SpaceSuffix.TXT": "ToolOutput Alpha\n",
                "nested/inside.txt": "ToolOutput alpha\n",
                "nested/.gitignore": "nested-ignored.txt\n",
                "nested/nested-ignored.txt": "ToolOutput alpha\n",
                ".gitignore": "ignored.txt\nignored-tracked.txt\nignored-dir/\n",
                ".ignore": "dot-ignored.txt\n",
                "ignored.txt": "ToolOutput alpha\n", "ignored-tracked.txt": "ToolOutput alpha\n",
                "ignored-dir/inside.txt": "ToolOutput alpha\n",
                "dot-ignored.txt": "ToolOutput alpha\n",
                "offline-delete.txt": "TGREP_SPIKE_OFFLINE_DELETE\n"}
    payloads["json-lines.txt"] = b"first line\nJSON_DIAGNOSTIC needle\r\nnext JSON_DIAGNOSTIC needle\n"
    for name, data in payloads.items():
        target = root / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data if isinstance(data, bytes) else data.encode())
    (root / "control\nname.txt").write_text("ToolOutput alpha\n")
    try:
        fd = os.open(os.fsencode(root) + b"/invalid-\xff.txt", os.O_WRONLY | os.O_CREAT, 0o600)
        os.close(fd)
    except OSError as exc:
        runner.report["fixture_notes"].append({"invalid_utf8_creation": type(exc).__name__})
    (root / "linked.txt").symlink_to("ordinary.txt")
    (root / "linked-dir").symlink_to("nested", target_is_directory=True)
    if git:
        runner.run(["/usr/bin/git", "-c", "init.templateDir=", "init", "-q", "."], root, allowed=(0,))
        runner.run(["/usr/bin/git", "add", "-f", "ordinary.txt", "ignored-tracked.txt"], root, allowed=(0,))
    return {"ordinary.txt", "untracked.txt", "empty.txt", "binary.bin", ".hidden.txt",
            "dir with spaces/Prefix SpaceSuffix.TXT", "nested/inside.txt", "nested/.gitignore",
            ".gitignore", ".ignore", "offline-delete.txt", "json-lines.txt"}


def build(runner, root, index, record):
    _, record["initial_build_ms"] = runner.run(
        runner.argv("tgrep", index, ["index", str(root), "--index-buffer", "256"]),
        root, timeout=120, allowed=(0,))
    record["index_bytes_after_build"] = disk_bytes(index)


def membership(runner, root, index, expected, record):
    record["expected_at_membership"] = sorted(expected)
    record["listing"] = []
    caches = {}
    variants = [(name, flags) for name, flags in POLICIES.items()]
    variants.append(("at_explicit_git_descendants", [*AT_FLAGS, "-g", "!.git/**"]))
    for name, flags in variants:
        entry = {"policy": name, "flags": flags, "results": {}}
        record["listing"].append(entry)
        observed = {}
        for tool in ("rg", "tgrep"):
            def measure(tool=tool):
                paths, ms = files(runner, root, index, tool, flags)
                observed[tool] = paths
                caches[name + ":" + tool] = adapter_paths(paths, root)
                entry["results"][tool] = {
                    "discovery_ms": ms, "raw_expected_parity": difference(expected, paths, True),
                    "adapter_expected_parity": difference(expected, adapter_paths(paths, root), True),
                    "raw_git_paths": sorted(p for p in paths if ".git" in Path(p).parts)}
            phase(runner.report, "membership:" + name + ":" + tool, measure)
        if len(observed) == 2:
            entry["raw_rg_tgrep_parity"] = difference(observed["rg"], observed["tgrep"], True)
    record["cached_filter"] = []
    for source, cache in caches.items():
        for needle in ("SpAcE", "INSIDE", "no_such_filename"):
            timings = []
            wanted = {p for p in expected if needle.lower() in p.lower()}
            for iteration in range(WARMUPS + runner.args.samples):
                runner.check()
                start = time.perf_counter()
                selected = {p for p in cache if needle.lower() in p.lower()}
                ms = (time.perf_counter() - start) * 1000
                if iteration >= WARMUPS:
                    timings.append(ms)
            record["cached_filter"].append({"source": source, "needle": needle,
                                            "selected_paths": sorted(selected),
                                            "parity": difference(wanted, selected, True),
                                            "full_paths_preserved": selected <= cache,
                                            **distribution(timings)})


def content_cases(runner, root, index, record):
    record["content"] = []
    cases = [("literal", "ToolOutput", ["-F"]), ("regex", r"alpha|Alpha", []),
             ("case", "alpha", ["-i"]), ("glob", "ToolOutput", ["-g", "*.txt"]),
             ("no_match", "TGREP_SPIKE_NOT_PRESENT_928371", ["-F"])]
    for policy, policy_flags in POLICIES.items():
        for name, pattern, flags in cases:
            entry = {"case": name, "policy": policy, "flags": [*policy_flags, *flags]}
            record["content"].append(entry)
            def compare():
                rg, _ = query(runner, root, index, "rg", pattern, entry["flags"])
                tg, _ = query(runner, root, index, "tgrep", pattern, entry["flags"])
                entry["parity"] = difference(rg, tg)
            phase(runner.report, "content:" + policy + ":" + name, compare)


def rpc_status(runner, process, index):
    runner.check()
    if process.poll() is not None:
        raise CommandError("owned_server_exited")
    info_path = index / "serve.json"
    if info_path.stat().st_size > 4096:
        raise SafetyError("oversized_server_discovery")
    info = json.loads(info_path.read_bytes())
    if info["pid"] != process.pid:
        raise SafetyError("server_discovery_pid_not_owned")
    if not isinstance(info["port"], int) or not 1 <= info["port"] <= 65535:
        raise SafetyError("invalid_server_port")
    with socket.create_connection(("127.0.0.1", info["port"]), timeout=0.25) as connection:
        connection.sendall(b'{"jsonrpc":"2.0","id":1,"method":"status","params":{}}\n')
        data = bytearray()
        deadline = time.monotonic() + 0.5
        while b"\n" not in data:
            runner.check()
            if time.monotonic() >= deadline:
                raise TimeoutError("status_deadline")
            chunk = connection.recv(4096)
            if not chunk:
                raise ConnectionError("status_eof")
            data.extend(chunk)
            if len(data) > 65536:
                raise SafetyError("oversized_RPC_status")
        response = json.loads(bytes(data).split(b"\n", 1)[0])
    if response.get("id") != 1 or "result" not in response:
        raise ValueError("invalid_status_response")
    status = response["result"]
    return {key: status[key] for key in ("indexing", "watcher_active", "num_files")}


def start_server(runner, root, index, record, sentinel=SENTINEL):
    argv = runner.argv("tgrep", index, ["serve", str(root), "--max-memory", "256", "--max-cpu", "25"])
    record["serve_argv"] = argv
    record["ready"] = False
    process = runner.start(argv, root, server=True)
    start = time.monotonic()
    deadline = min(start + 120, runner.deadline)
    record["readiness_observations"] = []
    while time.monotonic() < deadline:
        runner.check()
        if process.poll() is not None:
            raise CommandError("server_exited_before_ready")
        try:
            status = rpc_status(runner, process, index)
            observation = {"elapsed_ms": (time.monotonic() - start) * 1000, **status}
            record["readiness_observations"].append(observation)
            if status["indexing"] is False and status["watcher_active"] is True:
                actual, _ = query(runner, root, index, "tgrep", sentinel, ["-F"],
                                  timeout=min(30, max(0.01, deadline - time.monotonic())))
                observation["sentinel_matches"] = sum(actual.values())
                if actual:
                    record["ready"] = True
                    record["ready_ms"] = (time.monotonic() - start) * 1000
                    return process
        except (OSError, ValueError, KeyError):
            pass
        time.sleep(0.05)
    record["ready"] = False
    raise CommandError("readiness_120s_timeout")


def live_freshness(runner, root, index, process, record):
    record["live_mutations"] = []
    path = root / "live-freshness.txt"
    old, new = "TGREP_SPIKE_LIVE_OLD", "zqvxjk98264NEWCONTENT"
    for operation in ("create", "edit", "delete"):
        start = time.monotonic()
        if operation == "create":
            path.write_text(old + "\n")
        elif operation == "edit":
            path.write_text(new + "\n")
        else:
            path.unlink()
        entry = {"operation": operation, "attempts": [], "converged": False}
        record["live_mutations"].append(entry)
        deadline = start + 5
        # Disjoint literal tokens ensure stale old postings cannot satisfy the
        # edit by simply rereading the same file through an alternation candidate.
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise CommandError("server_exited_during_mutation")
            attempt = {}
            entry["attempts"].append(attempt)
            try:
                def remaining():
                    if time.monotonic() >= deadline:
                        raise CommandError("freshness_deadline")
                    return min(30, deadline - time.monotonic())
                content_equal = True
                attempt["content"] = {}
                for token in (old, new):
                    rg, _ = query(runner, root, index, "rg", token, ["-F"], timeout=remaining())
                    tg, _ = query(runner, root, index, "tgrep", token, ["-F"], timeout=remaining())
                    attempt["content"][token] = difference(rg, tg)
                    content_equal &= rg == tg
                rg_paths, _ = files(runner, root, index, "rg", timeout=remaining())
                tg_paths, _ = files(runner, root, index, "tgrep", timeout=remaining())
                expected_present = operation != "delete"
                filename_equal = ((path.name in rg_paths) == (path.name in tg_paths) == expected_present)
                sentinel, _ = query(runner, root, index, "tgrep", SENTINEL, ["-F"], timeout=remaining())
                status = rpc_status(runner, process, index)
                attempt.update(filename_present_rg=path.name in rg_paths,
                               filename_present_tgrep=path.name in tg_paths,
                               sentinel_matches=sum(sentinel.values()), **status)
                if (content_equal and filename_equal and sentinel and status["indexing"] is False
                        and status["watcher_active"] is True and time.monotonic() <= deadline):
                    entry.update(converged=True, freshness_ms=(time.monotonic() - start) * 1000)
                    break
            except (CommandError, OSError, ValueError, KeyError) as exc:
                attempt["error"] = type(exc).__name__
            time.sleep(min(0.05, max(0, deadline - time.monotonic())))
        entry["elapsed_ms"] = (time.monotonic() - start) * 1000


def json_diagnostics(runner, root, index, record):
    record["json_diagnostics"] = []
    for name, tgrep_flags in (("server_plain", []), ("server_byte_offset", ["-b"]),
                              ("no_index", ["--no-index"])):
        entry = {"variant": name, "tgrep_flags": tgrep_flags}
        record["json_diagnostics"].append(entry)
        def compare():
            rg, _ = query(runner, root, index, "rg", "JSON_DIAGNOSTIC", ["-F"])
            tg, _ = query(runner, root, index, "tgrep", "JSON_DIAGNOSTIC", ["-F", *tgrep_flags])
            entry["exact_parity"] = difference(rg, tg)
            for tool, rows in (("rg", rg), ("tgrep", tg)):
                entry[tool] = [{"line_number": row[1], "absolute_offset": row[2],
                                "CRLF": row[3].endswith(b"\r\n"),
                                "LF": row[3].endswith(b"\n"), "count": count}
                               for row, count in sorted(rows.items())]
        phase(runner.report, "json_diagnostic:" + name, compare)


def fixture_suite(runner, cases):
    report = runner.report
    report.setdefault("fixtures", {})
    for label, is_git in cases:
        root, index = runner.work / label, runner.index(label)
        record = report["fixtures"][label] = {}
        inherited_git = any(os.path.lexists(parent / ".git") for parent in root.parents)
        record["ancestor_git_detected"] = inherited_git
        if not is_git and inherited_git:
            # Only the sub-1KiB synthetic nonGit fixture may leave scratch.
            # Its index, captured output, HOME and all large corpora stay there.
            temporary = tempfile.TemporaryDirectory(prefix="tgrep-nongit-", dir=tempfile.gettempdir())
            runner.fixture_temps.append(temporary)
            root = Path(temporary.name).resolve() / "fixture"
            record["fallback"] = "owned system-temp fixture; all indexes and output remain in scratch"
            record["fallback_ancestor_git_detected"] = any(
                os.path.lexists(parent / ".git") for parent in root.parents)
            if record["fallback_ancestor_git_detected"]:
                record.update(status="blocked_not_tested", reason="system_temp_also_inside_git")
                return
        expected = fixture(runner, root, is_git)
        if not is_git:
            record["synthetic_fixture_bytes"] = disk_bytes(root)
            if record["synthetic_fixture_bytes"] >= 1024:
                raise SafetyError("nonGit_fixture_1KiB_exceeded")
        build(runner, root, index, record)
        membership(runner, root, index, expected, record)
        content_cases(runner, root, index, record)
        nested = record["nested_root"] = {}
        membership(runner, root / "nested", index, {"inside.txt", ".gitignore"}, nested)
        if not is_git:
            continue
        (root / "offline-create.txt").write_text("TGREP_SPIKE_OFFLINE_CREATE\n")
        (root / "offline-delete.txt").unlink()
        stale = record["offline_stale"] = {}
        for tool in ("rg", "tgrep"):
            paths, _ = files(runner, root, index, tool)
            stale[tool] = {"created_visible": "offline-create.txt" in paths,
                           "deleted_visible": "offline-delete.txt" in paths}
        for token in ("TGREP_SPIKE_OFFLINE_CREATE", "TGREP_SPIKE_OFFLINE_DELETE"):
            rg, _ = query(runner, root, index, "rg", token, ["-F"])
            tg, _ = query(runner, root, index, "tgrep", token, ["-F"])
            stale[token] = difference(rg, tg)
        before = set(runner.processes)
        try:
            process = phase(report, "fixture_server_ready", lambda: start_server(runner, root, index, record))
            if process is not None:
                json_diagnostics(runner, root, index, record)
                phase(report, "fixture_live_freshness",
                      lambda: live_freshness(runner, root, index, process, record))
        finally:
            for owned in set(runner.processes) - before:
                runner.finish(owned)
        fallback = record["stopped_server_fallback"] = {}
        (root / "stopped-create.txt").write_text("TGREP_SPIKE_STOPPED_CREATE\n")
        for tool in ("rg", "tgrep"):
            paths, _ = files(runner, root, index, tool)
            fallback[tool + "_listing"] = {"created_visible": "stopped-create.txt" in paths,
                                           "sentinel_file_visible": "ordinary.txt" in paths}
        for token in (SENTINEL, "TGREP_SPIKE_STOPPED_CREATE"):
            rg, _ = query(runner, root, index, "rg", token, ["-F"])
            tg, _ = query(runner, root, index, "tgrep", token, ["-F"])
            fallback[token] = difference(rg, tg)
        record["final_index_bytes"] = disk_bytes(index)


def benchmark_pair(runner, root, index, record, invoke):
    record["first_invocation"] = {}
    first = {}
    for tool in ("rg", "tgrep"):
        result, latency = invoke(tool)
        first[tool] = result
        record["first_invocation"][tool] = {"latency_ms": latency, "count": sum(result.values())
                                           if isinstance(result, Counter) else len(result)}
    record["pre_timing_parity"] = difference(first["rg"], first["tgrep"])
    samples = {tool: [] for tool in ("rg", "tgrep")}
    record["warmup_ms"] = {tool: [] for tool in samples}
    record["samples_ms"] = samples
    record["round_parity"] = []
    record["baseline_stable"] = True
    for iteration in range(WARMUPS + runner.args.samples):
        order = ("rg", "tgrep") if iteration % 2 == 0 else ("tgrep", "rg")
        outputs = {}
        for tool in order:
            outputs[tool], latency = invoke(tool)
            target = record["warmup_ms"] if iteration < WARMUPS else samples
            target[tool].append(latency)
        record["round_parity"].append(difference(outputs["rg"], outputs["tgrep"]))
        record["baseline_stable"] &= outputs["rg"] == first["rg"]
    record["latency"] = {tool: distribution(values) for tool, values in samples.items()}
    exact = (record["pre_timing_parity"]["equal"] and record["baseline_stable"]
             and all(item["equal"] for item in record["round_parity"]))
    record["exact_parity_all_rounds"] = exact
    if exact:
        record["rg_over_tgrep_median_ratio"] = statistics.median(samples["rg"]) / statistics.median(samples["tgrep"])
    else:
        record["speedup_withheld"] = "parity mismatch or changing baseline"


def benchmark(runner, label, root, sentinel=None):
    record = runner.report["benchmarks"][label] = {"cases": []}
    index = runner.index(label + "-benchmark")
    build(runner, root, index, record)
    before = set(runner.processes)
    try:
        if sentinel:
            start_server(runner, root, index, record, sentinel)
        else:
            # A known generated token cannot be injected into the read-only repo.
            # Require a nonempty rg result as the repo's existing sentinel oracle.
            rg, _ = query(runner, root, index, "rg", "ToolOutput", ["-F"])
            record["repo_sentinel_rg_count"] = sum(rg.values())
            if not rg:
                raise CommandError("repo_sentinel_absent")
            start_server(runner, root, index, record, "ToolOutput")
        patterns = [("broad", "ToolOutput")]
        if sentinel:
            patterns.insert(0, ("selective", sentinel))
        else:
            patterns.insert(0, ("selective_candidate", "TGREP_SPIKE"))
        paths_case = {"kind": "literal_case_sensitive_paths_only_CLI", "policy": "default_indexed",
                      "pattern": "ToolOutput", "boundary": "CLI, not FFF public-tool latency"}
        record["cases"].append(paths_case)
        phase(runner.report, label + ":paths_only",
              lambda: benchmark_pair(runner, root, index, paths_case,
                                     lambda tool: matching_files(runner, root, index, tool)))
        benchmark_policies = {"default_indexed": [], "at_git_descendants": SAFE_AT_FLAGS}
        for policy, flags in benchmark_policies.items():
            listing_record = {"kind": "filename_discovery_CLI", "policy": policy}
            record["cases"].append(listing_record)
            phase(runner.report, label + ":listing:" + policy,
                  lambda: benchmark_pair(runner, root, index, listing_record,
                                         lambda tool: files(runner, root, index, tool, flags)))
            for name, pattern in patterns:
                entry = {"kind": "content_CLI_spawn_and_IPC", "case": name,
                         "pattern": pattern, "policy": policy}
                record["cases"].append(entry)
                phase(runner.report, label + ":" + policy + ":" + name,
                      lambda: benchmark_pair(runner, root, index, entry,
                                             lambda tool: query(runner, root, index, tool, pattern, [*flags, "-F"])))
        cache, discovery = files(runner, root, index, "rg", SAFE_AT_FLAGS)
        cache = adapter_paths(cache, root)
        cached = record["cached_filename_substring"] = {"source": "adapter-filtered rg listing proxy",
                                                       "discovery_ms": discovery, "catalog_count": len(cache),
                                                       "cases": []}
        for needle in ("tool", "SRC", "file-04999", "no_such_filename"):
            values = []
            for iteration in range(WARMUPS + runner.args.samples):
                runner.check()
                start = time.perf_counter()
                selected = [path for path in cache if needle.lower() in path.lower()]
                elapsed = (time.perf_counter() - start) * 1000
                if iteration >= WARMUPS:
                    values.append(elapsed)
            cached["cases"].append({"needle": needle, "count": len(selected), **distribution(values)})
        record["index_bytes"] = disk_bytes(index)
    finally:
        for owned in set(runner.processes) - before:
            runner.finish(owned)


def generated(runner):
    root = runner.work / "generated"
    root.mkdir()
    runner.run(["/usr/bin/git", "-c", "init.templateDir=", "init", "-q", "."], root, allowed=(0,))
    total = 0
    token = "TGREP_SPIKE_UNIQUE_9f731a82"
    for number in range(5000):
        if number % 100 == 0:
            runner.check()
        data = ("ToolOutput shared benchmark line\n" +
                (token + "\n" if number == 4999 else "") + f"ordinal {number}\n").encode()
        data += (b"nonmatching padding 0123456789\n" * 150)[:4096 - len(data)]
        total += len(data)
        if total > 32 * 1024**2:
            raise SafetyError("generated_content_32MiB_exceeded")
        (root / f"file-{number:05d}.txt").write_bytes(data)
    runner.report["generated"] = {"files": 5000, "content_bytes": total,
                                  "unique_query": token, "broad_query": "ToolOutput"}
    benchmark(runner, "generated", root, token)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tgrep", type=Path, required=True, help="absolute verified v1.0.4 executable")
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="new JSON report outside searched repo")
    parser.add_argument("--scratch", type=Path, required=True)
    parser.add_argument("--samples", type=int, default=20)
    args = parser.parse_args()
    if not args.tgrep.is_absolute() or not args.tgrep.is_file() or not os.access(args.tgrep, os.X_OK):
        parser.error("--tgrep must be an absolute executable path")
    args.repo, args.output, args.scratch = (p.resolve() for p in (args.repo, args.output, args.scratch))
    if not args.repo.is_dir() or args.samples < 1:
        parser.error("--repo must be a directory and --samples must be positive")
    if within(args.scratch, args.repo) or within(args.output, args.repo):
        parser.error("scratch and output must be outside the searched repo")
    if args.output.exists():
        parser.error("--output must not already exist")
    args.scratch.mkdir(parents=True, exist_ok=True)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    report = {"schema_version": 1, "expected_release": "1.0.4", "samples": args.samples,
              "warmups": WARMUPS, "commands": [], "errors": [], "fixture_notes": [],
              "benchmarks": {}, "budgets": {"query_s": 30, "initial_build_s": 120,
              "total_s": 600, "output_bytes_per_command": OUTPUT_LIMIT,
              "combined_index_bytes": INDEX_LIMIT, "server_rss_bytes": RSS_LIMIT,
              "monitor_interval_s": 0.5, "generated_files": 5000, "generated_max_bytes": 32 * 1024**2},
              "interpretation": {
                  "filename_baseline": "rg --files listing proxy, NOT the true TUI walker",
                  "cached_filter": "in-process substring filtering, NOT CLI discovery or a TUI insertion test",
                  "latency": "CLI wall time includes spawn and IPC when delegated, not public-tool latency",
                  "default_indexed": "index-eligible flags; actual routing is not instrumented",
                  "at_compatible": "source-known index bypass: --hidden for content; --hidden and --no-require-git for files",
                  "git_exclusion": "raw !.git and !.git/** results retained separately from adapter filtering",
                  "FFF": "not measured: no production/public-tool adapter invoked; historical timings not reused",
                  "limits": "periodically monitored budgets, not hard OS resource limits",
                  "index_cpu": "index uses RAYON_NUM_THREADS=floor(cores/4), min 1; serve uses --max-cpu 25",
                  "source": "pinned v1.0.4 search.rs filename_index_compatible and bypass_index; serve.rs status",
                  "privacy": "no stdout, stderr, repository matches or repository path differences persisted"}}
    started = time.monotonic()
    runner = None
    try:
        with tempfile.TemporaryDirectory(prefix="tgrep-spike-", dir=args.scratch) as directory:
            work = Path(directory)
            report["temporary_workspace"] = str(work)
            runner = Runner(args, work, report)
            try:
                version, _ = runner.run([str(args.tgrep), "--version"], work, allowed=(0,))
                report["verified_version_1_0_4"] = version.strip() == b"tgrep 1.0.4"
                if not report["verified_version_1_0_4"]:
                    raise SafetyError("unexpected_tgrep_version")
                for label, is_git in (("git", True), ("non_git", False)):
                    phase(report, label + "_fixture", lambda: fixture_suite(runner, [(label, is_git)]))
                phase(report, "repo_benchmark", lambda: benchmark(runner, "repo", args.repo))
                phase(report, "generated_benchmark", lambda: generated(runner))
                runner.check(force=True)
            finally:
                runner.close()
    except BaseException as exc:
        report["errors"].append({"phase": "run", "type": type(exc).__name__,
                                 "reason": str(exc) if isinstance(exc, SafetyError) else "details withheld"})
    finally:
        report["elapsed_s"] = time.monotonic() - started
        report["execution_complete"] = not report["errors"]
        report["recommendations"] = {
            "at_selection": "retain current walker pending raw membership review and real TUI adapter validation",
            "agent_content": "retain current routes pending exact parity and latency review; outline/trace not evaluated"}
        # Exclusive creation prevents accidentally replacing a report or a source file.
        with args.output.open("x", encoding="utf-8") as output:
            json.dump(report, output, indent=2, ensure_ascii=True, allow_nan=False)
            output.write("\n")
    return 0 if report["execution_complete"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
