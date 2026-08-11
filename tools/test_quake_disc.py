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
SPEC = importlib.util.spec_from_file_location(
    "quake_disc", ROOT / "tools" / "quake_disc.py"
)
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
        self.psoxide = root / "psoxide"
        self.psoxide.mkdir()
        run("git", "init", "-q", cwd=self.psoxide)
        run("git", "config", "user.name", "Test", cwd=self.psoxide)
        run("git", "config", "user.email", "test@example.invalid", cwd=self.psoxide)
        (self.psoxide / "sdk.txt").write_text("pinned\n", encoding="ascii")
        run("git", "add", "sdk.txt", cwd=self.psoxide)
        run("git", "commit", "-q", "-m", "fixture", cwd=self.psoxide)
        self.psoxide_revision = run(
            "git", "rev-parse", "HEAD", cwd=self.psoxide
        ).stdout.strip()
        self.programs_stamp = root / "programs.psoxide-revision"
        self.programs_stamp.write_text(self.psoxide_revision + "\n", encoding="ascii")

        self.source = root / "quake"
        self.source.mkdir()
        run("git", "init", "-q", cwd=self.source)
        run("git", "config", "user.name", "Test", cwd=self.source)
        run("git", "config", "user.email", "test@example.invalid", cwd=self.source)
        (self.source / ".gitignore").write_text("dist/\n", encoding="ascii")
        (self.source / "tracked.txt").write_text("pinned\n", encoding="ascii")
        declaration = self.source / quake_disc.PSOXIDE_REV_FILE
        declaration.parent.mkdir(parents=True)
        declaration.write_text(
            f'const PSOXIDE_REV: &str = "{self.psoxide_revision}";\n', encoding="ascii"
        )
        run(
            "git", "add", ".gitignore", "tracked.txt", str(declaration), cwd=self.source
        )
        run("git", "commit", "-q", "-m", "fixture", cwd=self.source)
        self.revision = run("git", "rev-parse", "HEAD", cwd=self.source).stdout.strip()

        dist = self.source / "dist"
        dist.mkdir()
        self.cue = dist / "quake-psx.cue"
        self.bin = dist / "quake-psx.bin"
        image = bytearray(24 * quake_disc.SECTOR_BYTES)
        boot_at = quake_disc.BOOT_EXE_LBA * quake_disc.SECTOR_BYTES + 24
        image[boot_at : boot_at + len(quake_disc.PSX_EXE_MAGIC)] = (
            quake_disc.PSX_EXE_MAGIC
        )
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
            self.psoxide,
            self.programs_stamp,
            self.cue,
            self.revision,
            self.psoxide_revision,
            digest(self.cue),
            digest(self.bin),
        )

    def commit_quake_declaration(self, declaration: str) -> None:
        path = self.source / quake_disc.PSOXIDE_REV_FILE
        path.write_text(declaration, encoding="ascii")
        run("git", "add", str(path), cwd=self.source)
        run("git", "commit", "-q", "-m", "change declaration", cwd=self.source)
        self.revision = run("git", "rev-parse", "HEAD", cwd=self.source).stdout.strip()

    def advance_psoxide(self) -> None:
        (self.psoxide / "sdk.txt").write_text("next\n", encoding="ascii")
        run("git", "add", "sdk.txt", cwd=self.psoxide)
        run("git", "commit", "-q", "-m", "advance fixture", cwd=self.psoxide)
        self.psoxide_revision = run(
            "git", "rev-parse", "HEAD", cwd=self.psoxide
        ).stdout.strip()


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
        toc[numbers : numbers + 4] = (image_lba + quake_disc.BOOT_EXE_LBA).to_bytes(
            4, "little"
        )
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
                quake_disc.TOC_LBA + sector
            ) * quake_disc.SECTOR_BYTES + quake_disc.USER_DATA_AT
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
            self.assertEqual(verified.psoxide_revision, fixture.psoxide_revision)
            self.assertEqual(
                verified.declared_psoxide_revision, fixture.psoxide_revision
            )
            self.assertEqual(verified.bin_bytes, 24 * quake_disc.SECTOR_BYTES)

    def test_rejects_wrong_revision(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = QuakeFixture(Path(directory))
            with self.assertRaisesRegex(
                quake_disc.VerificationError, "revision mismatch"
            ):
                quake_disc.verify_quake(
                    fixture.source,
                    fixture.psoxide,
                    fixture.programs_stamp,
                    fixture.cue,
                    "0" * 40,
                    fixture.psoxide_revision,
                    digest(fixture.cue),
                    digest(fixture.bin),
                )

    def test_rejects_dirty_source(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = QuakeFixture(Path(directory))
            (fixture.source / "tracked.txt").write_text("changed\n", encoding="ascii")
            with self.assertRaisesRegex(
                quake_disc.VerificationError, "source checkout is dirty"
            ):
                fixture.verify()

    def test_rejects_quake_and_psoxide_revision_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = QuakeFixture(Path(directory))
            fixture.advance_psoxide()
            with self.assertRaisesRegex(
                quake_disc.VerificationError, "PSOXIDE_REV mismatch"
            ):
                fixture.verify()

    def test_rejects_dirty_psoxide_checkout(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = QuakeFixture(Path(directory))
            (fixture.psoxide / "sdk.txt").write_text("dirty\n", encoding="ascii")
            with self.assertRaisesRegex(
                quake_disc.VerificationError, "PSoXide checkout is dirty"
            ):
                fixture.verify()

    def test_rejects_malformed_quake_psoxide_revision(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = QuakeFixture(Path(directory))
            fixture.commit_quake_declaration(
                'const PSOXIDE_REV: &str = "not-a-full-revision";\n'
            )
            with self.assertRaisesRegex(
                quake_disc.VerificationError, "Quake PSOXIDE_REV"
            ):
                fixture.verify()

    def test_rejects_uppercase_quake_psoxide_revision(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = QuakeFixture(Path(directory))
            fixture.commit_quake_declaration(
                f'const PSOXIDE_REV: &str = "{fixture.psoxide_revision.upper()}";\n'
            )
            with self.assertRaisesRegex(
                quake_disc.VerificationError, "full lowercase hexadecimal"
            ):
                fixture.verify()

    def test_rejects_mismatched_ordinary_program_revision_stamp(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = QuakeFixture(Path(directory))
            fixture.programs_stamp.write_text("0" * 40 + "\n", encoding="ascii")
            with self.assertRaisesRegex(
                quake_disc.VerificationError, "ordinary-program SDK revision mismatch"
            ):
                fixture.verify()

    def test_rejects_changed_bin(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = QuakeFixture(Path(directory))
            expected_bin_hash = digest(fixture.bin)
            with fixture.bin.open("r+b") as stream:
                stream.seek(-1, 2)
                stream.write(b"x")
            with self.assertRaisesRegex(
                quake_disc.VerificationError, "bin SHA-256 mismatch"
            ):
                quake_disc.verify_quake(
                    fixture.source,
                    fixture.psoxide,
                    fixture.programs_stamp,
                    fixture.cue,
                    fixture.revision,
                    fixture.psoxide_revision,
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
                {
                    "demo_cue": str(demo_cue),
                    "demo_bin": str(demo_bin),
                    "out": str(receipt_path),
                },
            )()
            quake_disc.write_receipt(args, verified)
            receipt = json.loads(receipt_path.read_text(encoding="ascii"))
            self.assertEqual(
                receipt["quake_input"]["source_revision"], fixture.revision
            )
            self.assertTrue(receipt["quake_input"]["source_tree_clean"])
            self.assertEqual(
                receipt["quake_input"]["declared_psoxide_revision"],
                fixture.psoxide_revision,
            )
            self.assertEqual(
                receipt["psoxide_input"]["revision"], fixture.psoxide_revision
            )
            self.assertTrue(receipt["psoxide_input"]["tree_clean"])
            self.assertTrue(receipt["psoxide_input"]["matches_quake_declared_revision"])
            self.assertEqual(
                receipt["psoxide_input"]["ordinary_programs_revision"],
                fixture.psoxide_revision,
            )
            self.assertTrue(
                receipt["psoxide_input"]["ordinary_programs_match_checkout"]
            )
            self.assertEqual(
                receipt["quake_artifact_sdk_provenance"]["status"],
                "source-contract-only",
            )
            self.assertIn(
                "build sidecar",
                receipt["quake_artifact_sdk_provenance"]["required_follow_up"],
            )
            self.assertEqual(receipt["quake_input"]["bin_sha256"], digest(fixture.bin))
            self.assertEqual(
                receipt["demo_disc_output"]["bin_sha256"], digest(demo_bin)
            )
            self.assertTrue(
                receipt["demo_disc_output"]["embedded_quake_matches_input_except_msf"]
            )
            self.assertEqual(
                receipt["demo_disc_output"]["quake_toc"]["image_lba_offset"], 30
            )
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
                {
                    "demo_cue": str(demo_cue),
                    "demo_bin": str(demo_bin),
                    "out": str(root / "x.json"),
                },
            )()
            with self.assertRaisesRegex(
                quake_disc.VerificationError, "differs outside"
            ):
                quake_disc.write_receipt(args, verified)

    def test_receipt_rejects_mode_byte_drift(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = QuakeFixture(root)
            verified = fixture.verify()
            demo_cue, demo_bin = self.make_demo(root, fixture)
            with demo_bin.open("r+b") as stream:
                stream.seek(30 * quake_disc.SECTOR_BYTES + 15)
                stream.write(b"\x01")
            args = type(
                "Args",
                (),
                {
                    "demo_cue": str(demo_cue),
                    "demo_bin": str(demo_bin),
                    "out": str(root / "x.json"),
                },
            )()
            with self.assertRaisesRegex(
                quake_disc.VerificationError, "differs outside"
            ):
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
        self.assertIn('--version-of "QUAKE SHAREWARE=q1fd5173"', quake.stdout)
        self.assertIn('--describe "QUAKE SHAREWARE=', quake.stdout)
        self.assertIn("tools/quake_disc.py verify", quake.stdout)
        self.assertIn("tools/quake_disc.py receipt", quake.stdout)
        self.assertIn('--psoxide "', quake.stdout)
        self.assertIn('--programs-psoxide-stamp "', quake.stdout)
        self.assertIn('--expected-psoxide-revision "', quake.stdout)
        self.assertIn("PSoXide Demo Disc Quake Shareware.bin", quake.stdout)

    def test_full_quake_build_checks_sdk_coherence_after_programs(self) -> None:
        quake = run(
            "make",
            "-n",
            "--no-print-directory",
            "quake-disc",
            "DIST=/tmp/psoxide-quake-contract",
            cwd=ROOT,
            check=False,
        )
        self.assertEqual(quake.returncode, 0, quake.stderr)
        programs_at = quake.stdout.index("PSOXIDE_FROM=")
        coherence_at = quake.stdout.index('expected="local:')
        stamp_at = quake.stdout.index("programs.psoxide-revision.tmp")
        verify_at = quake.stdout.index("tools/quake_disc.py verify")
        self.assertLess(programs_at, coherence_at)
        self.assertLess(coherence_at, stamp_at)
        self.assertLess(stamp_at, verify_at)

    def test_quake_program_stamp_rejects_missing_malformed_and_stale(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = QuakeFixture(root)
            build = root / "build"
            stamp = build / "programs.psoxide-revision"

            def verify_stamp() -> subprocess.CompletedProcess[str]:
                return run(
                    "make",
                    "--no-print-directory",
                    "quake-programs-verify",
                    f"PSOXIDE={fixture.psoxide}",
                    f"BUILD={build}",
                    cwd=ROOT,
                    check=False,
                )

            missing = verify_stamp()
            self.assertNotEqual(missing.returncode, 0)
            self.assertIn("missing", missing.stdout)

            build.mkdir()
            stamp.write_text("not-a-revision\n", encoding="ascii")
            malformed = verify_stamp()
            self.assertNotEqual(malformed.returncode, 0)
            self.assertIn("malformed", malformed.stdout)

            stamp.write_text(fixture.psoxide_revision + "\n", encoding="ascii")
            matching = verify_stamp()
            self.assertEqual(matching.returncode, 0, matching.stderr)

            fixture.advance_psoxide()
            stale = verify_stamp()
            self.assertNotEqual(stale.returncode, 0)
            self.assertIn("but PSoXide is", stale.stdout)

    def test_headless_path_requires_the_program_revision_stamp(self) -> None:
        headless = run(
            "make",
            "-n",
            "--no-print-directory",
            "quake-headless-check",
            "DIST=/tmp/psoxide-quake-contract",
            cwd=ROOT,
            check=False,
        )
        self.assertEqual(headless.returncode, 0, headless.stderr)
        stamp_at = headless.stdout.index("programs.psoxide-revision")
        replay_at = headless.stdout.index("tools/check_quake_headless.py")
        self.assertLess(stamp_at, replay_at)

    def test_cortex_bakes_a_staged_copy_of_the_tracked_sample(self) -> None:
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn("$(PSOXIDE)/editor/samples/cortex_v1", makefile)
        self.assertNotIn("$(PSOXIDE)/editor/projects/cortex_v1", makefile)
        self.assertIn("CORTEX_PROJECT := $(BUILD)/cortex_v1", makefile)

    def test_half_life_and_quake_fail_closed(self) -> None:
        mixed = self.dry_run("HL=1", "QUAKE=1")
        self.assertNotEqual(mixed.returncode, 0)
        self.assertIn("QUAKE and HL are mutually exclusive", mixed.stderr)

    def test_distribution_targets_contain_explicit_quake_guards(self) -> None:
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn(
            "release-web: Quake shareware needs separate legal and release approval",
            makefile,
        )
        self.assertIn(
            "itch: Quake shareware needs separate legal and release approval", makefile
        )


if __name__ == "__main__":
    unittest.main()
