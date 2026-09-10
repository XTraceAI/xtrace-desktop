#!/usr/bin/env python3
"""Exercise the pinned production Stop hook with disposable data and loopback only.

The source snapshot is unchanged. Only its copied .mcp.json is configured for
the config-route case; the installed plugin, real HOME and host activity are unused.
"""
import argparse
import io
import json
import os
from pathlib import Path
import queue
import sqlite3
import subprocess
import sys
import tarfile
import tempfile
import threading


def plugin_snapshot(root, expected, destination):
    repo = Path(subprocess.check_output(["git", "-C", str(root), "rev-parse", "--show-toplevel"], text=True).strip())
    actual = subprocess.check_output(["git", "-C", str(repo), "rev-parse", "HEAD"], text=True).strip()
    if actual != expected or len(actual) != 40:
        raise RuntimeError("Plugin revision differs from the required pin")
    relative = root.relative_to(repo).as_posix()
    archive = subprocess.check_output(["git", "-C", str(repo), "archive", f"{actual}:{relative}"])
    with tarfile.open(fileobj=io.BytesIO(archive)) as source:
        for member in source.getmembers():
            target = destination / member.name
            if not target.resolve().is_relative_to(destination.resolve()):
                raise RuntimeError("Invalid snapshot member")
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            elif member.isfile():
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(source.extractfile(member).read())
            else:
                raise RuntimeError("Snapshot requires regular files")
    return actual


def start_server(binary, database):
    child = subprocess.Popen([str(binary), "serve", "--db", str(database), "--port", "0"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    try:
        lines = queue.Queue()
        threading.Thread(target=lambda: lines.put(child.stdout.readline()), daemon=True).start()
        line = lines.get(timeout=10).strip()
        if not line.startswith("XTRACE_PORT="):
            raise RuntimeError("Server did not report a port")
        return child, int(line.split("=", 1)[1])
    except BaseException:
        stop_server(child)
        raise


def stop_server(child):
    child.terminate()
    try:
        child.wait(timeout=5)
    except subprocess.TimeoutExpired:
        child.kill()
        child.wait()


def record(uuid, rich=False):
    message = {"role": "assistant"}
    if rich:
        message.update(model="synthetic-model", content=[{"type": "text", "text": "Synthetic result"}], usage={"input_tokens": 7, "output_tokens": 3})
    return {"uuid": uuid, "type": "assistant", "message": message}


def append(path, rows):
    with path.open("ab") as stream:
        for row in rows:
            stream.write((json.dumps(row) + "\n").encode())


def cursor(home):
    path = home / ".config" / "memhub-plugin" / "turnflush" / "synthetic-session.json"
    return json.loads(path.read_text()) if path.exists() else {}


# Invoke the production entry point under a network audit guard. This does not
# replace its transport, auth, cursor, filtering or acknowledgement code.
RUN_HOOK = """
import runpy, sys
port = int(sys.argv[2])
def audit(event, args):
    if event == 'socket.connect':
        address = args[1]
        if not isinstance(address, tuple) or address[:2] != ('127.0.0.1', port):
            raise RuntimeError('Conformance forbids non-test network connections')
sys.addaudithook(audit)
runpy.run_path(sys.argv[1], run_name='__main__')
"""


def exercise(binary, root, pin, directory, route):
    home = directory / "home"
    home.mkdir(parents=True)
    plugin = directory / "plugin"
    plugin_snapshot(root, pin, plugin)
    source_before = {p.relative_to(plugin): p.read_bytes() for p in plugin.rglob("*.py")}
    transcript = directory / "synthetic.jsonl"
    database = directory / "synthetic.db"
    child, port = start_server(binary, database)
    base = f"http://127.0.0.1:{port}"
    # Never inherit credentials, proxy settings, PYTHONPATH or host config roots.
    env = {"HOME": str(home), "PATH": os.defpath, "PYTHONNOUSERSITE": "1", "PYTHONDONTWRITEBYTECODE": "1", "MEMHUB_TOKEN": "local", "CLAUDE_PLUGIN_ROOT": str(plugin), "MEMHUB_TURN_FLUSH_TIMEOUT_S": "10", "NO_PROXY": "127.0.0.1,localhost,::1"}
    if route == "env":
        env["MEMHUB_MCP_BASE_URL"] = base
    else:
        (plugin / ".mcp.json").write_text(json.dumps({"mcpServers": {"memhub": {"url": base + "/mcp-server/mcp"}}}))

    def flush():
        result = subprocess.run([sys.executable, "-c", RUN_HOOK, str(plugin / "scripts" / "flush_turn.py"), str(port)], input=json.dumps({"session_id": "synthetic-session", "transcript_path": str(transcript)}), env=env, cwd=home, text=True, capture_output=True, timeout=15)
        assert result.returncode == 0, "Hook must exit quietly"
        return result.stdout

    def received(sql):
        return sql.execute("SELECT count(*) FROM capture_receipts").fetchone()[0]

    try:
        with sqlite3.connect(database) as sql:
            evidence = []
            append(transcript, [record("first"), {"type": "assistant", "message": {}}])
            flush()
            first = cursor(home)
            assert first["offset"] == transcript.stat().st_size and first["last_uuid"] == "first", "Initial durable ack must advance the real cursor"
            assert sql.execute("SELECT count(*) FROM records").fetchone()[0] == 1 and received(sql) == 1
            evidence.append({"case": "first", "before": 0, "after": first["offset"], "receipts": 1})
            append(transcript, [record("first", rich=True)])
            log = flush()
            enriched = cursor(home)
            assert enriched["offset"] == transcript.stat().st_size and enriched["offset"] > first["offset"]
            assert "new=0" in log, "Enrichment cannot count as a new UUID"
            assert sql.execute("SELECT output_tokens FROM usage").fetchone()[0] == 3 and received(sql) == 2
            evidence.append({"case": "enriched", "before": first["offset"], "after": enriched["offset"], "receipts": 2})
            append(transcript, [record("first", rich=True)])
            log = flush()
            duplicate = cursor(home)
            assert "new=0" in log and duplicate["offset"] == transcript.stat().st_size and received(sql) == 3
            evidence.append({"case": "duplicate", "before": enriched["offset"], "after": duplicate["offset"], "receipts": 3})
            sql.executescript("CREATE TABLE test_commit(parent TEXT REFERENCES sessions(session_id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER test_commit_failure AFTER INSERT ON capture_receipts BEGIN INSERT INTO test_commit VALUES('missing-parent'); END;")
            append(transcript, [record("retry", rich=True)])
            flush()
            failed = cursor(home)
            assert failed["offset"] == duplicate["offset"] and failed["last_uuid"] == duplicate["last_uuid"], "Failed COMMIT must leave the real cursor unchanged"
            assert failed["last_error"] == "server_rejected"
            assert received(sql) == 3 and sql.execute("SELECT count(*) FROM records WHERE uuid='retry'").fetchone()[0] == 0
            evidence.append({"case": "commit_failure", "before": duplicate["offset"], "after": failed["offset"], "receipts": 3})
            sql.execute("DROP TRIGGER test_commit_failure")
            sql.commit()
            flush()
            retried = cursor(home)
            assert retried["offset"] == transcript.stat().st_size and retried["last_uuid"] == "retry"
            assert not retried.get("last_error") and received(sql) == 4
            evidence.append({"case": "retry", "before": failed["offset"], "after": retried["offset"], "receipts": 4})
            assert sql.execute("SELECT session_id FROM sessions").fetchone()[0] == "synthetic-session"
            assert sql.execute("SELECT surface FROM capture_receipts LIMIT 1").fetchone()[0] is None
            assert sql.execute("PRAGMA quick_check").fetchone()[0] == "ok"
            rows = sql.execute("SELECT r.uuid,r.type,r.role,r.model,u.input_tokens,u.output_tokens FROM records r LEFT JOIN usage u ON u.uuid=r.uuid ORDER BY r.uuid").fetchall()
        assert all((plugin / path).read_bytes() == content for path, content in source_before.items()), "Pinned Python source must remain unchanged"
        print(json.dumps({"route": route, "pin": pin, "cursor_evidence": evidence}))
        return rows
    finally:
        stop_server(child)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--plugin-root", type=Path, required=True)
    parser.add_argument("--expected-commit", required=True)
    parser.add_argument("--binary", type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="xtrace-import-") as directory:
        root = Path(directory)
        env_rows = exercise(args.binary.resolve(), args.plugin_root.resolve(), args.expected_commit, root / "env", "env")
        config_rows = exercise(args.binary.resolve(), args.plugin_root.resolve(), args.expected_commit, root / "config", "config")
        assert env_rows == config_rows, "Both routing mechanisms must persist the same canonical rows"
    print("Pinned real Stop-hook import conformance passed through environment and plugin config routes.")


if __name__ == "__main__":
    main()
