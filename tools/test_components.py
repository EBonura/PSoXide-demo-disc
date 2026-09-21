import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import components


class GameCoherenceTests(unittest.TestCase):
    def test_rejects_transitive_sdk_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            specs = {name: {"revision": letter * 40, "path": name}
                     for name, letter in (("sdk", "a"), ("emulator", "b"), ("editor", "c"))}
            (root / "release-components.json").write_text(json.dumps({"schema": 1, "components": specs}))
            for owner, dependencies in (("editor", ("sdk", "emulator")), ("emulator", ("sdk",))):
                source = root / owner
                source.mkdir()
                (source / "components.lock.json").write_text(json.dumps({
                    "components": {name: specs[name] for name in dependencies}}))
            def git_result(source, *args):
                return specs[source.name]["revision"] if args[0] == "rev-parse" else ""
            with patch.object(components, "ROOT", root), patch.object(components, "git", side_effect=git_result), patch.object(components.subprocess, "run"):
                components.run(check=True)
                # The launcher now resolves SDK crates through the editor.
                # Reject drift in either importing owner before any build.
                for owner in ("editor", "emulator"):
                    with self.subTest(owner=owner):
                        path = root / owner / "components.lock.json"
                        original = path.read_text()
                        lock = json.loads(original)
                        lock["components"]["sdk"]["revision"] = "d" * 40
                        path.write_text(json.dumps(lock))
                        with self.assertRaisesRegex(RuntimeError, owner + " sdk pin differs"):
                            components.run(check=True)
                        path.write_text(original)

    def fixture(self, root):
        specs = {name: {"revision": letter * 40, "repository": "owner/" + name, "path": name}
                 for name, letter in (("sdk", "a"), ("emulator", "b"), ("editor", "c"))}
        (root / "release-components.json").write_text(json.dumps({"components": specs}))
        editor = root / "editor"
        (editor / "engine").mkdir(parents=True)
        (editor / "sdk").mkdir()
        (editor / "engine/source.rs").write_text("engine source")
        (editor / "sdk/source.rs").write_text("sdk source")
        (editor / ".components-receipt.json").write_text(json.dumps({"files": {
            "sdk/source.rs": hashlib.sha256(b"sdk source").hexdigest()}}))
        for game in (*components.GAMES, "hl-psx"):
            source = root / "games" / game
            source.mkdir(parents=True)
            (source / "components.lock.json").write_text(json.dumps({"components": specs}))
            if game == "hl-psx":
                continue
            hydrated = source / ".psoxide"
            (hydrated / "engine").mkdir(parents=True)
            (hydrated / "sdk").mkdir()
            (hydrated / "engine/source.rs").write_text("engine source")
            (hydrated / "sdk/source.rs").write_text("sdk source")
            (hydrated / ".psoxide-source").write_text(f"local:{editor}")

    def test_requires_every_game_and_unchanged_imports(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            with patch.object(components, "ROOT", root), patch.object(components, "git", return_value="engine/source.rs"):
                components.verify_games()
                with self.assertRaisesRegex(RuntimeError, "hl-psx: missing"):
                    components.verify_games(hl=True)
                source = root / "games/voxide/.psoxide/sdk/source.rs"
                source.write_text("tampered")
                with self.assertRaisesRegex(RuntimeError, "build input changed"):
                    components.verify_games()
                source.write_text("sdk source")
                (root / "games/voxide/.psoxide/.psoxide-source").unlink()
                with self.assertRaisesRegex(RuntimeError, "missing current"):
                    components.verify_games()

    def test_rejects_standalone_component_drift(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.fixture(root)
            path = root / "games/nitroxide/components.lock.json"
            lock = json.loads(path.read_text())
            lock["components"]["sdk"]["revision"] = "d" * 40
            path.write_text(json.dumps(lock))
            with patch.object(components, "ROOT", root):
                with self.assertRaisesRegex(RuntimeError, "standalone sdk revision"):
                    components.verify_game_locks()
