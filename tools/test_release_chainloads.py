from __future__ import annotations

import csv
import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("check_release_chainloads.py")
SPEC = importlib.util.spec_from_file_location("check_release_chainloads", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
chainloads = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = chainloads
SPEC.loader.exec_module(chainloads)


class CortexGameplayTests(unittest.TestCase):
    def write_gpu(self, rows: list[tuple[int, int, int, str]]) -> Path:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        path = Path(directory.name) / "gpu.csv"
        with path.open("w", newline="", encoding="ascii") as stream:
            writer = csv.writer(stream)
            writer.writerow(
                ("route_tick", "textured_tris", "textured_quads", "frame_draw_hash")
            )
            writer.writerows(rows)
        return path

    def write_display(self, textured: bool) -> Path:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        path = Path(directory.name) / "display.ppm"
        pixels = bytearray()
        for y in range(240):
            for x in range(320):
                in_oracle = 32 <= x < 288 and 36 <= y < 150
                value = 32 if textured and in_oracle and x % 2 == 0 else 96
                pixels.extend((value, value, value))
        path.write_bytes(b"P6\n320 240\n255\n" + pixels)
        return path

    def test_cortex_route_enters_internal_menu_after_launcher(self) -> None:
        self.assertEqual(
            chainloads.route_for("CORTEX IGNITION", 0, 12),
            "1000:cross:12,1400:cross:12,1800:cross:12,2200:cross:12,"
            "2600:cross:12,3000:cross:12,3400:cross:12",
        )
        self.assertEqual(
            chainloads.route_for("HALF-LIFE", 2, 12),
            "400:left:8,600:left:8,1000:cross:12,1400:cross:12,"
            "1800:cross:12,2200:cross:12,2600:cross:12,3000:cross:12,"
            "3400:cross:12",
        )

    def test_sustained_textured_gameplay_passes(self) -> None:
        rows = [
            (100 + index * 8, 700, 100, f"0x{index + 1:016x}")
            for index in range(35)
        ]
        evidence = chainloads.cortex_gameplay_evidence(self.write_gpu(rows))
        self.assertEqual(evidence, {"frames": 35, "sustained": 35, "hashes": 35})

    def test_half_life_train_ride_gameplay_passes(self) -> None:
        rows = [
            (100 + index * 4, 423, 246, f"0x{index + 1:016x}")
            for index in range(35)
        ]
        evidence = chainloads.hl_gameplay_evidence(self.write_gpu(rows))
        self.assertEqual(evidence, {"frames": 35, "sustained": 35, "hashes": 35})

    def test_menu_like_frames_fail(self) -> None:
        path = self.write_gpu([(index, 0, 31, "0x1") for index in range(100)])
        with self.assertRaisesRegex(chainloads.CheckError, "only 0 textured"):
            chainloads.cortex_gameplay_evidence(path)

    def test_isolated_frames_are_not_sustained(self) -> None:
        rows = [(index * 32, 700, 100, f"0x{index + 1:x}") for index in range(35)]
        path = self.write_gpu(rows)
        with self.assertRaisesRegex(chainloads.CheckError, "not sustained"):
            chainloads.cortex_gameplay_evidence(path)

    def test_static_frame_hash_fails(self) -> None:
        rows = [(index * 8, 700, 100, "0x1") for index in range(35)]
        path = self.write_gpu(rows)
        with self.assertRaisesRegex(chainloads.CheckError, "did not animate"):
            chainloads.cortex_gameplay_evidence(path)

    def test_textured_upper_playfield_passes_geometry_oracle(self) -> None:
        self.assertGreaterEqual(
            chainloads.cortex_geometry_evidence(self.write_display(textured=True)),
            chainloads.CORTEX_GEOMETRY_MIN_EDGE_PERMILLE,
        )

    def test_flat_upper_playfield_fails_geometry_oracle(self) -> None:
        with self.assertRaisesRegex(chainloads.CheckError, "lacks textured geometry"):
            chainloads.cortex_geometry_evidence(self.write_display(textured=False))


if __name__ == "__main__":
    unittest.main()
