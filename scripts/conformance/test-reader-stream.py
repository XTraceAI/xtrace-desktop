#!/usr/bin/env python3
"""Run the pinned read-only Codex/Cursor reader stream over synthetic native files.

The producer is the exact reviewed source named by .plugin-pin: the checkout
must be at the pinned commit, every listed reader source must be the pinned
Git object, and the plugin root is snapshotted from Git before it runs. The
readers see a disposable HOME built from the fixture catalog's native inputs,
may not open any network connection, and their stream must match the fixture's
hand-written identity expectations and the committed contract golden exactly.
Nothing here reads the real home directory or a live host.
"""
import argparse
import difflib
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import sys
import tarfile
import tempfile
import uuid
from bundle_source import bundle_mode, snapshot_bundle, verify_bundle

UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
HEADER_KEYS = {"type", "host", "native_session_id", "conversation_id", "source_surface",
               "started_at", "cwd", "git_branch", "title", "path", "mtime"}
IDENTITY_KEYS = ("native_session_id", "source_surface", "started_at", "cwd", "git_branch")


class Mismatch(Exception):
    """A clear, bounded contract failure; never echoes transcript content."""


def read_pin(path):
    pin = json.loads(Path(path).read_text(encoding="utf-8"))
    if not re.fullmatch(r"[0-9a-f]{40}", pin["commit"]):
        raise Mismatch(".plugin-pin commit must be a full SHA")
    return pin


def git(repo, *args):
    return subprocess.check_output(["git", "-C", str(repo), *args], text=True,
                                   stderr=subprocess.DEVNULL).strip()


def verify_sources(repo, pin):
    """HEAD and every pinned reader source must be the exact pinned objects."""
    head = git(repo, "rev-parse", "HEAD")
    if head != pin["commit"]:
        raise Mismatch("plugin checkout is not at the pinned commit")
    for path, expected in pin["reader_sources"].items():
        try:
            actual = git(repo, "rev-parse", "--verify", f"HEAD:{path}")
        except subprocess.CalledProcessError:
            raise Mismatch(f"pinned reader source is absent: {path}") from None
        if actual != expected:
            raise Mismatch(f"pinned reader source differs from the pin: {path}")
    if git(repo, "status", "--porcelain"):
        raise Mismatch("plugin checkout has local modifications")
    return head


def snapshot(root, pin, destination, pin_path=None):
    if bundle_mode():
        return snapshot_bundle(root, destination, pin_path=pin_path)
    repo = Path(git(root, "rev-parse", "--show-toplevel"))
    head = verify_sources(repo, pin)
    relative = Path(root).resolve().relative_to(repo.resolve()).as_posix()
    if relative != pin["plugin_root"]:
        raise Mismatch("plugin root does not match the pinned plugin_root")
    archive = subprocess.check_output(["git", "-C", str(repo), "archive", f"{head}:{relative}"])
    with tarfile.open(fileobj=io.BytesIO(archive)) as source:
        for member in source.getmembers():
            target = destination / member.name
            if not target.resolve().is_relative_to(destination.resolve()):
                raise Mismatch("invalid snapshot member")
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            elif member.isfile():
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(source.extractfile(member).read())
            else:
                raise Mismatch("snapshot requires regular files")
    return head


def varint(value):
    out = b""
    while True:
        low, value = value & 0x7F, value >> 7
        out += bytes([low | (0x80 if value else 0)])
        if not value:
            return out


def build_store(spec, path):
    """Materialize a Cursor store.db from its JSON description.

    Mirrors the observed native format: JSON leaf blobs addressed by sha256, tree
    nodes listing child hashes (protobuf field 1), an optional checkpoint clock
    (field 26 varint, milliseconds) and a root recorded in the meta table.
    """
    leaves = [json.dumps(message).encode() for message in spec["messages"]]
    blobs = {hashlib.sha256(data).hexdigest(): data for data in leaves}
    leaf_ids = list(blobs)
    named = {}
    root = None
    for node in spec["nodes"]:
        data = b""
        for child in node["children"]:
            digest = named[child] if isinstance(child, str) else leaf_ids[child]
            data += b"\x0a\x20" + bytes.fromhex(digest)
        if node.get("clock_ms") is not None:
            data += b"\xd0\x01" + varint(int(node["clock_ms"]))
        data += bytes.fromhex(node.get("extra_hex", ""))
        digest = hashlib.sha256(data).hexdigest()
        blobs[digest] = data
        named[node["name"]] = digest
        root = digest
    path.parent.mkdir(parents=True, exist_ok=True)
    connection = sqlite3.connect(path)
    try:
        connection.execute("CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB)")
        connection.execute("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT)")
        connection.executemany("INSERT INTO blobs VALUES (?, ?)", list(blobs.items()))
        connection.execute("INSERT INTO meta VALUES ('0', ?)", (json.dumps(
            {"agentId": spec["agent_id"], "latestRootBlobId": root, "name": spec.get("name", "New Agent")}),))
        connection.commit()
    finally:
        connection.close()


def materialize(spec, home):
    anchor = int(spec["anchor_epoch"])
    for entry in spec["files"]:
        target = home / entry["path"]
        if not target.resolve().is_relative_to(home.resolve()):
            raise Mismatch("fixture path escapes the disposable home")
        target.parent.mkdir(parents=True, exist_ok=True)
        if "lines" in entry:
            target.write_text("".join(json.dumps(line) + "\n" for line in entry["lines"]), encoding="utf-8")
        elif "json" in entry:
            target.write_text(json.dumps(entry["json"]), encoding="utf-8")
        elif "store" in entry:
            build_store(entry["store"], target)
        else:
            raise Mismatch("fixture entry declares no content")
    for path in home.rglob("*"):
        os.utime(path, (anchor, anchor))


# The pinned CLI runs as itself; only an audit hook forbids any network use.
RUN_CLI = """
import os, runpy, sys
def audit(event, args):
    if event == 'socket.connect':
        raise RuntimeError('reader conformance forbids network connections')
sys.addaudithook(audit)
sys.argv = [sys.argv[1]] + sys.argv[2:]
sys.path[0] = os.path.dirname(sys.argv[0])   # as `python readers_cli.py` would resolve its siblings
runpy.run_path(sys.argv[0], run_name='__main__')
"""


def run_cli(plugin, home, host, mode):
    env = {"HOME": str(home), "USERPROFILE": str(home), "XDG_CONFIG_HOME": str(home / ".config"),
           "CODEX_HOME": str(home / ".codex"), "PATH": os.defpath, "PYTHONNOUSERSITE": "1",
           "PYTHONDONTWRITEBYTECODE": "1", "PYTHONUTF8": "1", "NO_PROXY": "*"}
    arguments = ["--host", host] + (["--metadata-only"] if mode == "metadata-only" else [])
    result = subprocess.run([sys.executable, "-c", RUN_CLI, str(plugin / "scripts" / "readers_cli.py"), *arguments],
                            env=env, cwd=home, text=True, capture_output=True, timeout=120)
    if result.returncode != 0 or result.stderr.strip():
        # Diagnostics are static JSON codes; a crash is summarized by its last line only.
        tail = [line for line in result.stderr.splitlines() if line.strip()][-1:]
        raise Mismatch(f"{host} {mode}: reader exited {result.returncode}; stderr: {tail[0][:160] if tail else ''}")
    return result.stdout.splitlines()


def normalize(lines, home):
    normalized = []
    for line in lines:
        row = json.loads(line)
        if row.get("type") == "session":
            path = Path(row["path"])
            row["path"] = "$HOME/" + path.relative_to(home.resolve()).as_posix()
        normalized.append(json.dumps(row, sort_keys=True))
    return normalized


def check_stream(lines, spec, host, mode):
    """Hand-written identity expectations and the shared record shape."""
    rows = [json.loads(line) for line in lines]
    headers = [row for row in rows if row.get("type") == "session"]
    records = [row for row in rows if row.get("type") != "session"]
    expected = [item for item in spec["expected_headers"] if item["host"] == host]
    if [h["native_session_id"] for h in headers] != [e["native_session_id"] for e in expected]:
        raise Mismatch(f"{host} {mode}: session headers differ from the fixture expectation")
    counts = {}
    current = None
    for row in rows:
        if row.get("type") == "session":
            current = row["native_session_id"]
            counts[current] = 0
            if set(row) != HEADER_KEYS or row["host"] != host or row["conversation_id"] != f"{host}-{current}":
                raise Mismatch(f"{host} {mode}: session header shape changed")
            continue
        counts[current] += 1
    for header, item in zip(headers, expected):
        for key in IDENTITY_KEYS:
            if header[key] != item[key]:
                raise Mismatch(f"{host} {mode}: {key} differs for {item['native_session_id']}")
        if mode == "full" and counts[item["native_session_id"]] != item["records"]:
            raise Mismatch(f"{host} {mode}: canonical record count differs for {item['native_session_id']}")
        if mode == "metadata-only" and counts[item["native_session_id"]]:
            raise Mismatch(f"{host} {mode}: metadata-only export emitted records")
    for record in records:
        if record.get("type") not in {"user", "assistant"} or not UUID.match(str(record.get("uuid", ""))):
            raise Mismatch(f"{host} {mode}: canonical record lacks a type or UUID")
        message = record.get("message")
        if not isinstance(message, dict) or message.get("role") != record["type"]:
            raise Mismatch(f"{host} {mode}: canonical record message role differs from its type")
    return len(headers), len(records)


def compare_golden(golden, key, normalized):
    if key not in golden:
        raise Mismatch(f"{key}: contract golden has no entry")
    if golden[key] != normalized:
        diff = list(difflib.unified_diff(golden[key], normalized, "pinned golden", "actual", lineterm="", n=0))
        first = next((line for line in diff[2:] if line[:1] in "+-"), "")
        raise Mismatch(f"{key}: reader stream differs from the pinned contract golden "
                       f"({len(diff)} diff lines; first: {first[:160]})")


def exercise(plugin, fixtures, *, write_golden=False, output=None):
    evidence = []
    for spec_path in sorted(fixtures.glob("F*/input/native/native.json")):
        spec = json.loads(spec_path.read_text(encoding="utf-8"))
        golden_path = spec_path.with_name(spec["golden"])
        golden = json.loads(golden_path.read_text(encoding="utf-8")) if golden_path.exists() else {}
        with tempfile.TemporaryDirectory(prefix="xtrace-reader-home-") as directory:
            home = Path(directory).resolve()
            materialize(spec, home)
            for run in spec["runs"]:
                host, mode = run["host"], run["mode"]
                lines = run_cli(plugin, home, host, mode)
                headers, records = check_stream(lines, spec, host, mode)
                normalized = normalize(lines, home)
                key = f"{host} {mode}"
                if write_golden:
                    golden[key] = normalized
                else:
                    compare_golden(golden, key, normalized)
                evidence.append({"fixture": spec_path.parts[-4], "host": host, "mode": mode,
                                 "headers": headers, "records": records, "lines": lines})
        if write_golden:
            golden_path.write_text(json.dumps(golden, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    if not evidence:
        raise Mismatch("no native fixture catalog entries were found")
    if output is not None:
        output.write_text(json.dumps({"runs": evidence}), encoding="utf-8")
    return evidence


def self_test(plugin, fixtures, pin, repository):
    """An incompatible reader change and a differing pinned object must fail clearly."""
    with tempfile.TemporaryDirectory(prefix="xtrace-reader-mutant-") as directory:
        mutant = Path(directory) / "plugin"
        shutil.copytree(plugin, mutant)
        cli = mutant / "scripts" / "readers_cli.py"
        text = cli.read_text(encoding="utf-8")
        # A quiet contract change: the header still exports, but under a different identity format.
        marker = '"conversation_id": f"{reader.HOST}-{sid}"'
        if marker not in text:
            raise Mismatch("self-test could not locate the header field to mutate")
        cli.write_text(text.replace(marker, '"conversation_id": f"{reader.HOST}:{sid}"', 1), encoding="utf-8")
        try:
            exercise(mutant, fixtures)
        except Mismatch as error:
            print(f"self-test: incompatible reader change fails clearly: {error}")
        else:
            raise Mismatch("self-test failed: an incompatible reader change was not detected")
    # The real checkout, at the pinned commit, against a pin that names a different
    # object for one reader source: only the specific identity mismatch may result.
    altered = dict(pin, reader_sources={**pin["reader_sources"], "plugins/memhub/scripts/readers_cli.py": "0" * 40})
    try:
        if bundle_mode():
            with tempfile.TemporaryDirectory(prefix="xtrace-reader-pin-") as directory:
                path = Path(directory) / "pin.json"
                path.write_text(json.dumps(altered), encoding="utf-8")
                verify_bundle(repository, path)
        else:
            verify_sources(repository, altered)
    except (Mismatch, subprocess.CalledProcessError) as error:
        reason = error.stderr if isinstance(error, subprocess.CalledProcessError) else str(error)
        if ("differs" if bundle_mode() else "differs from the pin") not in reason:
            raise Mismatch(f"self-test failed: unexpected source verification error: {error}") from None
    else:
        raise Mismatch("self-test failed: a differing pinned reader object was not detected")
    print("self-test: a differing pinned reader object fails source verification")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--plugin-root", type=Path, required=True)
    parser.add_argument("--pin", type=Path, required=True)
    parser.add_argument("--fixtures", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--write-golden", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--materialize", type=Path,
                        help="lay out one fixture's native inputs under this home directory and exit")
    parser.add_argument("--only", default="F18", help="fixture id for --materialize")
    args = parser.parse_args()
    if args.materialize is not None:
        spec_path = args.fixtures.resolve() / args.only / "input" / "native" / "native.json"
        materialize(json.loads(spec_path.read_text(encoding="utf-8")), args.materialize.resolve())
        print(f"materialized {args.only} native inputs under {args.materialize}")
        return 0
    pin = read_pin(args.pin)
    try:
        with tempfile.TemporaryDirectory(prefix="xtrace-reader-plugin-") as directory:
            plugin = Path(directory) / "plugins" / "memhub" if bundle_mode() else Path(directory) / "plugin"
            plugin.mkdir(parents=True)
            plugin_root = args.plugin_root.resolve()
            head = snapshot(plugin_root, pin, plugin, args.pin.resolve())
            evidence = exercise(plugin, args.fixtures.resolve(), write_golden=args.write_golden,
                                output=args.output.resolve() if args.output else None)
            if args.self_test:
                repository = plugin_root if bundle_mode() else Path(git(plugin_root, "rev-parse", "--show-toplevel"))
                self_test(plugin, args.fixtures.resolve(), pin, repository)
    except Mismatch as error:
        print(f"Pinned reader stream conformance failed: {error}", file=sys.stderr)
        return 1
    for item in evidence:
        print(f"{item['fixture']} {item['host']} {item['mode']}: {item['headers']} sessions, {item['records']} records")
    print(f"Pinned reader stream conformance passed at {head} ({len(evidence)} runs).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
