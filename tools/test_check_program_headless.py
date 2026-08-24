import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location(
    "check_program_headless", ROOT / "tools" / "check_program_headless.py"
)
assert SPEC is not None and SPEC.loader is not None
check = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = check
SPEC.loader.exec_module(check)


class ProgramHeadlessTests(unittest.TestCase):
    def test_parse_entries_and_menu_route(self):
        toc = bytearray(check.TOC_SECTORS * check.USER_DATA_BYTES)
        toc[:8] = check.TOC_MAGIC
        toc[8:12] = (3).to_bytes(4, "little")
        names = ("CORTEX", "VOXIDE", "NITROXIDE")
        for index, name in enumerate(names):
            at = check.TOC_HEADER_BYTES + index * check.TOC_ENTRY_BYTES
            toc[at : at + len(name)] = name.encode("ascii")
            toc[at + check.TOC_NAME_BYTES + 8 : at + check.TOC_NAME_BYTES + 12] = (
                index + 4
            ).to_bytes(4, "little")
        toc[check.TOC_HEADER_BYTES + 504 : check.TOC_HEADER_BYTES + 508] = (
            check.FLAG_HIDDEN
        ).to_bytes(4, "little")

        entries = check.parse_entries(bytes(toc))
        self.assertTrue(entries[0].hidden)
        self.assertEqual(entries[2].cdda_track_base, 6)
        presses, cross = check.menu_presses(entries, "NITROXIDE")
        self.assertEqual(presses, ["400:left:8", "1000:cross:12"])
        self.assertEqual(cross, 1000)

    def test_live_ppm_rejects_blank_frame(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "blank.ppm"
            path.write_bytes(b"P6\n320 240\n255\n" + bytes(320 * 240 * 3))
            with self.assertRaisesRegex(ValueError, "blank or visually implausible"):
                check.require_live_ppm(path)

    def test_live_ppm_accepts_coloured_frame(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "live.ppm"
            pixels = bytearray()
            for index in range(320 * 240):
                pixels.extend((index & 255, (index // 3) & 255, (index // 7) & 255))
            path.write_bytes(b"P6\n320 240\n255\n" + pixels)
            check.require_live_ppm(path)


if __name__ == "__main__":
    unittest.main()
