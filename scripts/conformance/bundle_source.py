"""Test-only access to the app's existing Rust bundle verifier, before imports."""
import json
import os
from pathlib import Path
import shutil
import subprocess


def bundle_mode():
    mode = os.environ.get("AGENT_PLUGINS_SOURCE", "checkout")
    if mode not in {"checkout", "bundle"}:
        raise RuntimeError("AGENT_PLUGINS_SOURCE must be checkout or bundle")
    return mode == "bundle"


def verify_bundle(plugin, pin_path=None, expected=None):
    pin_path = Path(pin_path or Path(__file__).resolve().parents[2] / ".plugin-pin")
    pin = json.loads(pin_path.read_text(encoding="utf-8"))
    if expected is not None and pin["commit"] != expected:
        raise RuntimeError("Bundle revision differs from the required pin")
    # Check the spelling before resolving so a symlink at the scripts tree
    # reaches the verifier, which refuses it rather than following it.
    plugin = Path(os.path.abspath(plugin))
    relative = Path(pin["plugin_root"])
    root = plugin.parents[len(relative.parts) - 1]
    if root / relative != plugin:
        raise RuntimeError("Bundle plugin root differs from the pin")
    verifier = os.environ.get("XTRACE_CONFORMANCE_BUNDLE_VERIFIER")
    if not verifier:
        raise RuntimeError("Bundle conformance requires the Rust bundle verifier")
    result = subprocess.run([verifier, str(pin_path), str(root)],
                            text=True, capture_output=True, check=True)
    if result.stdout.strip() != pin["commit"]:
        raise RuntimeError("Bundle verifier returned a different revision")
    return pin["commit"]


def snapshot_bundle(plugin, destination, pin_path=None, expected=None):
    head = verify_bundle(plugin, pin_path, expected)
    shutil.copytree(plugin / "scripts", destination / "scripts", copy_function=shutil.copy2)
    # Verify the copied tree too, before any producer can run from it.
    verify_bundle(destination, pin_path, expected)
    return head
