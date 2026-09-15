import json
from pathlib import Path
import tempfile
import unittest

from deploy_public import WEB_RECEIPT, web_record, verify_web_build


class WebBuildReceiptTests(unittest.TestCase):
    def test_requires_matching_source_and_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "frontend.wasm").write_bytes(b"wasm fixture")
            with self.assertRaisesRegex(ValueError, "no emulator source"):
                verify_web_build(root, "a" * 40, "b" * 64)
            (root / WEB_RECEIPT).write_text(json.dumps(web_record(root, "a" * 40, "b" * 64)))
            verify_web_build(root, "a" * 40, "b" * 64)
            with self.assertRaisesRegex(ValueError, "differs"):
                verify_web_build(root, "c" * 40, "b" * 64)
            (root / "frontend.wasm").write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "differs"):
                verify_web_build(root, "a" * 40, "b" * 64)
