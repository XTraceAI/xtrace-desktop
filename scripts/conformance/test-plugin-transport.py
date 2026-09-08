#!/usr/bin/env python3
"""Run the real pinned plugin decoder/client against a fresh local headless server.
No account credentials, host discovery, plugin config or saved activity are used.
"""
import argparse
import importlib
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import urllib.request


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--plugin-root", type=Path, required=True)
    parser.add_argument("--expected-commit", required=True)
    parser.add_argument("--binary", type=Path, required=True)
    args = parser.parse_args()
    root = args.plugin_root.resolve()
    repo = Path(subprocess.check_output(["git", "-C", str(root), "rev-parse", "--show-toplevel"], text=True).strip())
    actual = subprocess.check_output(["git", "-C", str(repo), "rev-parse", "HEAD"], text=True).strip()
    if actual != args.expected_commit or len(actual) != 40:
        raise RuntimeError("Plugin commit does not match the expected revision")
    for name in ["pak.py", "mcp_http.py", "atomic_write.py"]:
        path = root / "scripts" / name
        if path.is_symlink() or not path.resolve().is_relative_to(repo):
            raise RuntimeError("Plugin source path is not a regular checkout file")
        expected = subprocess.check_output(["git", "-C", str(repo), "show", actual + ":" + path.relative_to(repo).as_posix()])
        if path.read_bytes() != expected:
            raise RuntimeError("Plugin source differs from its pinned revision")
    os.environ["NO_PROXY"] = "127.0.0.1,localhost,::1"
    sys.path.insert(0, str(root / "scripts"))
    pak = importlib.import_module("pak")
    mcp = importlib.import_module("mcp_http")
    with tempfile.TemporaryDirectory(prefix="xtrace-transport-") as directory:
        child = subprocess.Popen([str(args.binary.resolve()), "serve", "--db", str(Path(directory) / "synthetic.db"), "--port", "0"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        try:
            lines = queue.Queue()
            threading.Thread(target=lambda: lines.put(child.stdout.readline()), daemon=True).start()
            line = lines.get(timeout=10).strip()
            if not line.startswith("XTRACE_PORT="):
                raise RuntimeError("Server did not report its bound port")
            port = int(line.split("=", 1)[1])
            base = "http://127.0.0.1:" + str(port)
            endpoint = base + "/mcp-server/mcp"
            response = mcp.request(endpoint, "synthetic", "initialize", {"protocolVersion": mcp.PROTOCOL_VERSION, "capabilities": {}, "clientInfo": {"name": "Synthetic conformance", "version": "1"}})
            assert response["protocolVersion"] == mcp.PROTOCOL_VERSION
            tools = mcp.list_tools(endpoint, "synthetic")
            assert len(tools) == 1 and tools[0]["name"] == "import_conversation"
            minted = pak.mint(base, "synthetic", "Synthetic transport test")
            assert minted["secret"] == minted["id"] == "local"
            assert minted["label"] == "Synthetic transport test"
            assert len(pak.list_keys(base, "synthetic")) == 1
            pak.revoke(base, "synthetic", "local")
            assert len(pak.list_keys(base, "synthetic")) == 1
            stub = mcp.call_tool(endpoint, "local", "import_conversation", {"conversation_id": "synthetic", "messages": [], "source_platform": "claude"})
            assert stub.isError and "not implemented" in stub.content[0].text
            notification = urllib.request.Request(endpoint, data=json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}).encode(), headers={"Content-Type": "application/json"})
            with urllib.request.urlopen(notification, timeout=5) as result:
                assert result.status == 202 and not result.read()
            for headers, expected in [({}, 404), ({"Origin": "https://example.invalid"}, 403)]:
                probe = urllib.request.Request(base + "/__e2e", headers=headers)
                try:
                    urllib.request.urlopen(probe, timeout=5)
                    raise AssertionError("Unexpected debug route success")
                except urllib.error.HTTPError as error:
                    assert error.code == expected
            print("Pinned real plugin transport passed: initialize, SSE tools/list, token mint/list/delete, notification, honest import stub.")
            print("Plugin revision: " + actual)
        finally:
            child.terminate()
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()


if __name__ == "__main__":
    try:
        main()
    except Exception:
        print("Plugin transport conformance failed; inspect the local test inputs.", file=sys.stderr)
        sys.exit(1)
