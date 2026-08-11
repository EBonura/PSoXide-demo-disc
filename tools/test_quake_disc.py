from __future__ import annotations

import hashlib
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location("quake_disc", ROOT / "tools" / "quake_disc.py")
assert SPEC is not None and SPEC.loader is not None
quake_disc = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = quake_disc
SPEC.loader.exec_module(quake_disc)


def run(*args: str, cwd: Path, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        list(args),
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=check,
    )


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


class QuakeFixture:
    def __init__(self, root: Path) -> None:
        self.source = root / "quake"
        self.source.mkdir()
        run("git", "init", "-q", cwd=self.source)
        run("git", "config", "user.name", "Test", cwd=self.source)
        run("git", "config", "user.email", "test@example.invalid", cwd=self.source)
        (self.source / ".gitignore").write_text("dist/\n", encoding="ascii")
        (self.source / "tracked.txt").write_text("pinned\n", encoding="ascii")
        run("git", "add", ".gitignore", "tracked.txt", cwd=self.source)
        run("git", "commit", "-q", "-m", "fixture", cwd=self.source)
        self.revision = run("git", "rev-parse", "HEAD", cwd=self.source).stdout.strip()

        dist = self.source / "dist"
        dist.mkdir()
        self.cue = dist / "quake-psx.cue"
        self.bin = dist / "quake-psx.bin"
        image = bytearray(24 * quake_disc.SECTOR_BYTES)
        boot_at = quake_disc.BOOT_EXE_LBA * quake_disc.SECTOR_BYTES + 24
        image[boot_at : boot_at + len(quake_disc.PSX_EXE_MAGIC)] = quake_disc.PSX_EXE_MAGIC
        self.bin.write_bytes(image)
        self.cue.write_text(
            'FILE "quake-psx.bin" BINARY\n'
            "  TRACK 01 MODE2/2352\n"
            "    INDEX 01 00:00:00\n",
            encoding="ascii",
        )

    def verify(self) -> quake_disc.VerifiedQuake:
        return quake_disc.verify_quake(
            self.source,
            self.cue,
            self.revision,
            digest(self.cue),
            digest(self.bin),
        )


class VerifyQuakeTests(unittest.TestCase):
    def make_demo(self, root: Path, fixture: QuakeFixture) -> tuple[Path, Path]:
        image_lba = 30
        input_image = fixture.bin.read_bytes()
        sectors = len(input_image) // quake_disc.SECTOR_BYTES
        image = bytearray((image_lba + sectors) * quake_disc.SECTOR_BYTES)
        image[image_lba * quake_disc.SECTOR_BYTES :] = input_image

        toc = bytearray(quake_disc.TOC_SECTORS * quake_disc.USER_DATA_BYTES)
        toc[:8] = quake_disc.TOC_MAGIC
        toc[8:12] = (1).to_bytes(4, "little")
        at = quake_disc.TOC_HEADER_BYTES
        toc[at : at + len(b"QUAKE SHAREWARE")] = b"QUAKE SHAREWARE"
        numbers = at + quake_disc.TOC_NAME_BYTES
        toc[numbers : numbers + 4] = (image_lba + quake_disc.BOOT_EXE_LBA).to_bytes(4, "little")
        toc[numbers + 4 : numbers + 8] = image_lba.to_bytes(4, "little")
        toc[numbers + 8 : numbers + 12] = (4).to_bytes(4, "little")
        toc[numbers + 12 : numbers + 16] = (0x12345678).to_bytes(4, "little")
        description_at = numbers + 16
        description = b"Pinned local test build"
        toc[description_at : description_at + len(description)] = description
        version_at = description_at + 2 * quake_disc.TOC_DESC_BYTES
        version = ("q" + fixture.revision[:7]).encode("ascii")
        toc[version_at : version_at + len(version)] = version
        for sector in range(quake_disc.TOC_SECTORS):
            source_at = sector * quake_disc.USER_DATA_BYTES
            output_at = (
                (quake_disc.TOC_LBA + sector) * quake_disc.SECTOR_BYTES
                + quake_disc.USER_DATA_AT
            )
            image[output_at : output_at + quake_disc.USER_DATA_BYTES] = toc[
                source_at : source_at + quake_disc.USER_DATA_BYTES
            ]

        demo_bin = root / "demo.bin"
        demo_cue = root / "demo.cue"
        demo_bin.write_bytes(image)
        demo_cue.write_text(
            'FILE "demo.bin" BINARY\n'
            "  TRACK 01 MODE2/2352\n"
            "    INDEX 01 00:00:00\n",
            encoding="ascii",
        )
        return demo_cue, demo_bin

    def test_accepts_exact_clean_revision_and_image(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = QuakeFixture(Path(directory))
            verified = fixture.verify()
            self.assertEqual(verified.source_revision, fixture.revision)
            self.assertEqual(verified.bin_bytes, 24 * quake_disc.SECTOR_BYTES)

    def test_rejects_wrong_revision(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = QuakeFixture(Path(directory))
            with self.assertRaisesRegex(quake_disc.VerificationError, "revision mismatch"):
                quake_disc.verify_quake(
                    fixture.source,
                    fixture.cue,
                    "0" * 40,
                    digest(fixture.cue),
                    digest(fixture.bin),
                )

    def test_rejects_dirty_source(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = QuakeFixture(Path(directory))
            (fixture.source / "tracked.txt").write_text("changed\n", encoding="ascii")
            with self.assertRaisesRegex(quake_disc.VerificationError, "source checkout is dirty"):
                fixture.verify()

    def test_rejects_changed_bin(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = QuakeFixture(Path(directory))
            expected_bin_hash = digest(fixture.bin)
            with fixture.bin.open("r+b") as stream:
                stream.seek(-1, 2)
                stream.write(b"x")
            with self.assertRaisesRegex(quake_disc.VerificationError, "bin SHA-256 mismatch"):
                quake_disc.verify_quake(
                    fixture.source,
                    fixture.cue,
                    fixture.revision,
                    digest(fixture.cue),
                    expected_bin_hash,
                )

    def test_rejects_cue_that_escapes_its_directory(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = QuakeFixture(Path(directory))
            fixture.cue.write_text(
                'FILE "../quake-psx.bin" BINARY\n'
                "  TRACK 01 MODE2/2352\n"
                "    INDEX 01 00:00:00\n",
                encoding="ascii",
            )
            with self.assertRaisesRegex(quake_disc.VerificationError, "beside the cue"):
                fixture.verify()

    def test_receipt_hashes_input_and_output_and_keeps_legal_gate(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = QuakeFixture(root)
            verified = fixture.verify()
            demo_cue, demo_bin = self.make_demo(root, fixture)
            receipt_path = root / "receipt.json"
            args = type(
                "Args",
                (),
                {"demo_cue": str(demo_cue), "demo_bin": str(demo_bin), "out": str(receipt_path)},
            )()
            quake_disc.write_receipt(args, verified)
            receipt = json.loads(receipt_path.read_text(encoding="ascii"))
            self.assertEqual(receipt["quake_input"]["source_revision"], fixture.revision)
            self.assertEqual(receipt["quake_input"]["bin_sha256"], digest(fixture.bin))
            self.assertEqual(receipt["demo_disc_output"]["bin_sha256"], digest(demo_bin))
            self.assertTrue(receipt["demo_disc_output"]["embedded_quake_matches_input_except_msf"])
            self.assertEqual(receipt["demo_disc_output"]["quake_toc"]["image_lba_offset"], 30)
            self.assertIn("legal and release approval", receipt["redistribution"])

    def test_receipt_rejects_embedded_image_drift(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = QuakeFixture(root)
            verified = fixture.verify()
            demo_cue, demo_bin = self.make_demo(root, fixture)
            with demo_bin.open("r+b") as stream:
                stream.seek(30 * quake_disc.SECTOR_BYTES + 100)
                stream.write(b"x")
            args = type(
                "Args",
                (),
                {"demo_cue": str(demo_cue), "demo_bin": str(demo_bin), "out": str(root / "x.json")},
            )()
            with self.assertRaisesRegex(quake_disc.VerificationError, "differs outside"):
                quake_disc.write_receipt(args, verified)


class MakeVariantContractTests(unittest.TestCase):
    def dry_run(self, *assignments: str) -> subprocess.CompletedProcess[str]:
        return run(
            "make",
            "-n",
            "--no-print-directory",
            "disc-only",
            "DIST=/tmp/psoxide-quake-contract",
            *assignments,
            cwd=ROOT,
            check=False,
        )

    def test_default_and_zero_do_not_add_quake(self) -> None:
        default = self.dry_run()
        zero = self.dry_run("QUAKE=0")
        self.assertEqual(default.returncode, 0, default.stderr)
        self.assertEqual(zero.returncode, 0, zero.stderr)
        self.assertEqual(default.stdout, zero.stdout)
        self.assertNotIn('--image "QUAKE SHAREWARE=', default.stdout)
        self.assertNotIn("tools/quake_disc.py", default.stdout)

    def test_half_life_and_zero_do_not_add_quake(self) -> None:
        half_life = self.dry_run("HL=1")
        zero = self.dry_run("HL=1", "QUAKE=0")
        self.assertEqual(half_life.returncode, 0, half_life.stderr)
        self.assertEqual(zero.returncode, 0, zero.stderr)
        self.assertEqual(half_life.stdout, zero.stdout)
        self.assertIn('--image "HALF-LIFE=', half_life.stdout)
        self.assertNotIn('--image "QUAKE SHAREWARE=', half_life.stdout)

    def test_opt_in_adds_whole_image_metadata_verifier_and_receipt(self) -> None:
        quake = self.dry_run("QUAKE=1")
        self.assertEqual(quake.returncode, 0, quake.stderr)
        self.assertIn('--image "QUAKE SHAREWARE=', quake.stdout)
        self.assertIn('--version-of "QUAKE SHAREWARE=q8d5a681"', quake.stdout)
        self.assertIn('--describe "QUAKE SHAREWARE=', quake.stdout)
        self.assertIn("tools/quake_disc.py verify", quake.stdout)
        self.assertIn("tools/quake_disc.py receipt", quake.stdout)
        self.assertIn("PSoXide Demo Disc Quake Shareware.bin", quake.stdout)

    def test_half_life_and_quake_fail_closed(self) -> None:
        mixed = self.dry_run("HL=1", "QUAKE=1")
        self.assertNotEqual(mixed.returncode, 0)
        self.assertIn("QUAKE and HL are mutually exclusive", mixed.stderr)

    def test_distribution_targets_contain_explicit_quake_guards(self) -> None:
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn(
            'release-web: Quake shareware needs separate legal and release approval', makefile
        )
        self.assertIn('itch: Quake shareware needs separate legal and release approval', makefile)


if __name__ == "__main__":
    unittest.main()
