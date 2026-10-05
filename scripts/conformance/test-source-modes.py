#!/usr/bin/env python3
"""Real bundle verification rejects bad inputs before any harness launches code.

Run after building the conformance_bundle example, with its absolute path in
XTRACE_CONFORMANCE_BUNDLE_VERIFIER. All inputs are temporary synthetic mutations.
"""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from bundle_source import bundle_mode, verify_bundle

ROOT = Path(__file__).resolve().parents[2]


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class SourceModes(unittest.TestCase):
    def test_source_mode_is_explicit_and_defaults_to_checkout(self):
        with patch.dict(os.environ, {}, clear=True):
            self.assertFalse(bundle_mode())
            os.environ["AGENT_PLUGINS_SOURCE"] = "checkout"
            self.assertFalse(bundle_mode())
            os.environ["AGENT_PLUGINS_SOURCE"] = "bundle"
            self.assertTrue(bundle_mode())
            os.environ["AGENT_PLUGINS_SOURCE"] = "typo"
            with self.assertRaisesRegex(RuntimeError, "checkout or bundle"):
                bundle_mode()

    def test_verified_bundle_does_not_need_git(self):
        plugin = ROOT / "vendor/agent-plugins/plugins/memhub"
        pin = json.loads((ROOT / ".plugin-pin").read_text())
        with patch.dict(os.environ, {"PATH": "", "AGENT_PLUGINS_SOURCE": "bundle"}):
            self.assertEqual(verify_bundle(plugin), pin["commit"])

    def test_invalid_bundles_fail_before_all_three_harnesses_execute(self):
        reader, stop, transport = [load(name) for name in
                                   ["test-reader-stream", "test-plugin-import", "test-plugin-transport"]]
        for mutation in ["byte", "mode", "extra", "symlink", "bytecode", "wrong-pin"]:
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory(prefix="xtrace-source-test-") as directory:
                root = Path(directory)
                bundle = root / "bundle"
                shutil.copytree(ROOT / "vendor/agent-plugins", bundle)
                plugin = bundle / "plugins/memhub"
                scripts = plugin / "scripts"
                pin_path = root / ".plugin-pin"
                pin = json.loads((ROOT / ".plugin-pin").read_text())
                if mutation == "byte":
                    with (scripts / "readers_cli.py").open("ab") as stream:
                        stream.write(b"\n# synthetic mutation\n")
                elif mutation == "mode":
                    file = scripts / "readers_cli.py"
                    file.chmod(file.stat().st_mode ^ 0o111)
                elif mutation == "extra":
                    (scripts / "unexpected.py").write_text("synthetic")
                elif mutation == "symlink":
                    file = scripts / "readers_cli.py"
                    file.unlink()
                    file.symlink_to(ROOT / "vendor/agent-plugins/plugins/memhub/scripts/readers_cli.py")
                elif mutation == "bytecode":
                    (scripts / "__pycache__").mkdir()
                    (scripts / "__pycache__/synthetic.pyc").write_bytes(b"synthetic")
                else:
                    pin["reader_sources"]["plugins/memhub/scripts"] = "0" * 40
                pin_path.write_text(json.dumps(pin))
                with patch.dict(os.environ, {"AGENT_PLUGINS_SOURCE": "bundle"}):
                    with self.assertRaises(subprocess.CalledProcessError):
                        verify_bundle(plugin, pin_path)
                    # Reader-stream runs its verifier before exercise().
                    args = ["harness", "--plugin-root", str(plugin), "--pin", str(pin_path),
                            "--fixtures", str(ROOT / "fixtures")]
                    with patch.object(sys, "argv", args), patch.object(reader, "exercise") as execute:
                        with self.assertRaises(subprocess.CalledProcessError):
                            reader.main()
                        execute.assert_not_called()
                    # Stop verifies even before starting its disposable server.
                    with patch.object(stop, "start_server") as start:
                        with self.assertRaises(subprocess.CalledProcessError):
                            stop.exercise(root / "unused-binary", plugin, pin["commit"], root / "stop", "env", pin_path)
                        start.assert_not_called()
                    # Transport verifies before importing any producer module.
                    args = ["harness", "--plugin-root", str(plugin), "--expected-commit", pin["commit"],
                            "--binary", str(root / "unused-binary"), "--pin", str(pin_path)]
                    with patch.object(sys, "argv", args), patch.object(transport.importlib, "import_module") as imports:
                        with self.assertRaises(subprocess.CalledProcessError):
                            transport.main()
                        imports.assert_not_called()


if __name__ == "__main__":
    sys.dont_write_bytecode = True
    unittest.main()
