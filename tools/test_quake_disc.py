from __future__ import annotations

import hashlib
import importlib.util
import json
import re
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

HEADLESS_SPEC = importlib.util.spec_from_file_location(
    "check_quake_headless", ROOT / "tools" / "check_quake_headless.py"
)
assert HEADLESS_SPEC is not None and HEADLESS_SPEC.loader is not None
check_quake_headless = importlib.util.module_from_spec(HEADLESS_SPEC)
sys.modules[HEADLESS_SPEC.name] = check_quake_headless
HEADLESS_SPEC.loader.exec_module(check_quake_headless)


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
        self.exe = dist / "quake-psx.exe"
        self.provenance = dist / "quake-psx.provenance.json"
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
        self.exe.write_bytes(b"PS-X EXE\0fixture")
        self.write_provenance()
        self.provenance_sha256 = digest(self.provenance)

    def provenance_document(self) -> dict[str, object]:
        return {
            "schema": quake_disc.QUAKE_PROVENANCE_SCHEMA,
            "quake_source": {"revision": self.revision, "tree_clean": True},
            "psoxide": {
                "revision": self.psoxide_revision,
                "tree_clean": True,
                "source_kind": quake_disc.PSOXIDE_SOURCE_KIND,
            },
            "shareware": {
                "pak0_sha256": quake_disc.SHAREWARE_PAK_SHA256,
                "pak0_bytes": quake_disc.SHAREWARE_PAK_BYTES,
            },
            "build": {
                "guest_stage_schema": quake_disc.GUEST_STAGE_SCHEMA,
                "guest_recipe_sha256": "1" * 64,
                "rust_toolchain_sha256": "2" * 64,
                "rustc_version": "rustc fixture\nrelease: fixture",
                "cargo_version": "cargo fixture\nrelease: fixture",
                "profile": quake_disc.BUILD_PROFILE,
                "features": [],
            },
            "artifacts": {
                "cue": {
                    "file": self.cue.name,
                    "sha256": digest(self.cue),
                    "bytes": self.cue.stat().st_size,
                },
                "bin": {
                    "file": self.bin.name,
                    "sha256": digest(self.bin),
                    "bytes": self.bin.stat().st_size,
                },
                "exe": {
                    "file": self.exe.name,
                    "sha256": digest(self.exe),
                    "bytes": self.exe.stat().st_size,
                },
            },
        }

    def write_provenance(self, document: dict[str, object] | None = None) -> None:
        if document is None:
            document = self.provenance_document()
        self.provenance.write_text(
            json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="ascii"
        )

    def verify(self) -> quake_disc.VerifiedQuake:
        return quake_disc.verify_quake(
            self.source,
            self.psoxide,
            self.programs_stamp,
            self.cue,
            self.provenance,
            self.revision,
            self.psoxide_revision,
            self.provenance_sha256,
            digest(self.cue),
            digest(self.bin),
            digest(self.exe),
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
        description = b"Pinned Quake shareware Episode 1"
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

    def test_accepts_separate_clean_sdk_for_ordinary_programs(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = QuakeFixture(root)
            programs_psoxide = root / "programs-psoxide"
            programs_psoxide.mkdir()
            run("git", "init", "-q", cwd=programs_psoxide)
            run("git", "config", "user.name", "Test", cwd=programs_psoxide)
            run(
                "git",
                "config",
                "user.email",
                "test@example.invalid",
                cwd=programs_psoxide,
            )
            (programs_psoxide / "sdk.txt").write_text(
                "shared runtime\n", encoding="ascii"
            )
            run("git", "add", "sdk.txt", cwd=programs_psoxide)
            run("git", "commit", "-q", "-m", "fixture", cwd=programs_psoxide)
            programs_revision = run(
                "git", "rev-parse", "HEAD", cwd=programs_psoxide
            ).stdout.strip()
            fixture.programs_stamp.write_text(
                programs_revision + "\n", encoding="ascii"
            )

            verified = quake_disc.verify_quake(
                fixture.source,
                fixture.psoxide,
                fixture.programs_stamp,
                fixture.cue,
                fixture.provenance,
                fixture.revision,
                fixture.psoxide_revision,
                fixture.provenance_sha256,
                digest(fixture.cue),
                digest(fixture.bin),
                digest(fixture.exe),
                programs_psoxide=programs_psoxide,
                expected_programs_psoxide_revision=programs_revision,
            )

            self.assertEqual(verified.psoxide_revision, fixture.psoxide_revision)
            self.assertEqual(verified.programs_psoxide_revision, programs_revision)

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
                    fixture.provenance,
                    "0" * 40,
                    fixture.psoxide_revision,
                    fixture.provenance_sha256,
                    digest(fixture.cue),
                    digest(fixture.bin),
                    digest(fixture.exe),
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
                    fixture.provenance,
                    fixture.revision,
                    fixture.psoxide_revision,
                    fixture.provenance_sha256,
                    digest(fixture.cue),
                    expected_bin_hash,
                    digest(fixture.exe),
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

    def test_rejects_missing_malformed_and_duplicate_key_sidecars(self) -> None:
        cases = (
            ("missing", None, "sidecar does not exist"),
            ("malformed", "{", "cannot parse"),
            ("duplicate", '{"schema": 1, "schema": 1}', "duplicate key"),
        )
        for label, contents, error in cases:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                fixture = QuakeFixture(Path(directory))
                if contents is None:
                    fixture.provenance.unlink()
                else:
                    fixture.provenance.write_text(contents, encoding="ascii")
                with self.assertRaisesRegex(quake_disc.VerificationError, error):
                    fixture.verify()

    def test_rejects_sidecar_not_beside_cue_with_exact_name(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = QuakeFixture(Path(directory))
            alternate = fixture.cue.parent / "shipping.json"
            fixture.provenance.replace(alternate)
            fixture.provenance = alternate
            with self.assertRaisesRegex(
                quake_disc.VerificationError, "must be beside the cue and named"
            ):
                fixture.verify()

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = QuakeFixture(root)
            elsewhere = root / "elsewhere"
            elsewhere.mkdir()
            alternate = elsewhere / fixture.provenance.name
            fixture.provenance.replace(alternate)
            fixture.provenance = alternate
            with self.assertRaisesRegex(
                quake_disc.VerificationError, "must be beside the cue and named"
            ):
                fixture.verify()

    def test_rejects_stale_sidecar_revisions_and_source_kind(self) -> None:
        cases = (
            (
                "quake revision",
                lambda document: document["quake_source"].__setitem__(
                    "revision", "0" * 40
                ),
                "provenance Quake revision mismatch",
            ),
            (
                "psoxide revision",
                lambda document: document["psoxide"].__setitem__("revision", "0" * 40),
                "provenance PSoXide revision mismatch",
            ),
            (
                "source kind",
                lambda document: document["psoxide"].__setitem__(
                    "source_kind", "remote"
                ),
                "source kind",
            ),
            (
                "dirty claim",
                lambda document: document["quake_source"].__setitem__(
                    "tree_clean", False
                ),
                "clean Quake source tree",
            ),
        )
        for label, mutate, error in cases:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                fixture = QuakeFixture(Path(directory))
                document = fixture.provenance_document()
                mutate(document)
                fixture.write_provenance(document)
                with self.assertRaisesRegex(quake_disc.VerificationError, error):
                    fixture.verify()

    def test_accepts_pinned_psoxide_hydration(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = QuakeFixture(Path(directory))
            document = fixture.provenance_document()
            document["psoxide"]["source_kind"] = "pinned_hydration"
            fixture.write_provenance(document)
            fixture.provenance_sha256 = digest(fixture.provenance)
            fixture.verify()

    def test_rejects_wrong_shareware_and_nonshipping_build_contracts(self) -> None:
        cases = (
            (
                "pak hash",
                lambda document: document["shareware"].__setitem__(
                    "pak0_sha256", "0" * 64
                ),
                "canonical Quake 1.06 shareware",
            ),
            (
                "pak size",
                lambda document: document["shareware"].__setitem__("pak0_bytes", 1),
                "canonical Quake 1.06 shareware",
            ),
            (
                "recipe missing",
                lambda document: document["build"].pop("guest_recipe_sha256"),
                "missing required field",
            ),
            (
                "toolchain malformed",
                lambda document: document["build"].__setitem__(
                    "rust_toolchain_sha256", "not-a-hash"
                ),
                "Rust toolchain SHA-256",
            ),
            (
                "profile",
                lambda document: document["build"].__setitem__("profile", "dev"),
                "release profile",
            ),
            (
                "features",
                lambda document: document["build"].__setitem__(
                    "features", ["emulator-telemetry"]
                ),
                "release profile",
            ),
        )
        for label, mutate, error in cases:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                fixture = QuakeFixture(Path(directory))
                document = fixture.provenance_document()
                mutate(document)
                fixture.write_provenance(document)
                with self.assertRaisesRegex(quake_disc.VerificationError, error):
                    fixture.verify()

    def test_rejects_artifact_name_size_hash_and_byte_drift(self) -> None:
        cases = (
            (
                "exe basename",
                lambda fixture, document: document["artifacts"]["exe"].__setitem__(
                    "file", fixture.bin.name
                ),
                "names .* expected",
            ),
            (
                "cue size",
                lambda fixture, document: document["artifacts"]["cue"].__setitem__(
                    "bytes", fixture.cue.stat().st_size + 1
                ),
                "byte-size mismatch",
            ),
            (
                "bin sidecar hash",
                lambda fixture, document: document["artifacts"]["bin"].__setitem__(
                    "sha256", "0" * 64
                ),
                "artifacts.bin SHA-256 mismatch",
            ),
            (
                "exe bytes",
                lambda fixture, document: fixture.exe.write_bytes(b"changed"),
                "artifacts.exe.*mismatch",
            ),
        )
        for label, mutate, error in cases:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                fixture = QuakeFixture(Path(directory))
                document = fixture.provenance_document()
                mutate(fixture, document)
                fixture.write_provenance(document)
                with self.assertRaisesRegex(quake_disc.VerificationError, error):
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
            self.assertTrue(
                receipt["psoxide_input"]["ordinary_programs_match_quake_sdk"]
            )
            self.assertEqual(
                receipt["quake_artifact_sdk_provenance"]["status"],
                "sidecar-bound",
            )
            self.assertEqual(receipt["schema"], 3)
            self.assertEqual(
                receipt["quake_artifact_sdk_provenance"]["build"][
                    "guest_recipe_sha256"
                ],
                "1" * 64,
            )
            self.assertEqual(receipt["quake_input"]["bin_sha256"], digest(fixture.bin))
            self.assertEqual(receipt["quake_input"]["exe_sha256"], digest(fixture.exe))
            self.assertEqual(
                receipt["quake_input"]["provenance_sha256"],
                digest(fixture.provenance),
            )
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
    @staticmethod
    def pinned_menu_version() -> str:
        """The menu version the disc derives from the Quake pin.

        Read from the Makefile rather than hard coded: a repin is a routine
        event, and a literal here turns every repin into a spurious test
        failure instead of a real one.
        """
        for line in (ROOT / "Makefile").read_text().splitlines():
            if line.startswith("QUAKE_EXPECTED_REV"):
                return "q" + line.split("=", 1)[1].strip()[:7]
        raise AssertionError("Makefile has no QUAKE_EXPECTED_REV")

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

    def test_default_disc_carries_quake_with_metadata_verifier_and_receipt(
        self,
    ) -> None:
        default = self.dry_run()
        self.assertEqual(default.returncode, 0, default.stderr)
        self.assertIn('--image "QUAKE SHAREWARE=', default.stdout)
        self.assertIn(
            f'--version-of "QUAKE SHAREWARE={self.pinned_menu_version()}"',
            default.stdout,
        )
        self.assertIn('--describe "QUAKE SHAREWARE=', default.stdout)
        self.assertIn("tools/quake_disc.py verify", default.stdout)
        self.assertIn("tools/quake_disc.py receipt", default.stdout)
        self.assertIn('--psoxide "', default.stdout)
        self.assertIn('--programs-psoxide "', default.stdout)
        self.assertIn('--programs-psoxide-stamp "', default.stdout)
        self.assertIn('--expected-psoxide-revision "', default.stdout)
        self.assertIn('--expected-programs-psoxide-revision "', default.stdout)
        self.assertIn('--provenance "', default.stdout)
        self.assertIn('--expected-provenance-sha256 "', default.stdout)
        self.assertIn('--expected-exe-sha256 "', default.stdout)
        self.assertIn("PSoXide Demo Disc.bin", default.stdout)
        self.assertNotIn("PSoXide Demo Disc Quake Shareware.bin", default.stdout)

    def test_the_opt_in_switch_is_gone(self) -> None:
        default = self.dry_run()
        for assignment in ("QUAKE=", "QUAKE=0", "QUAKE=1"):
            with self.subTest(assignment=assignment):
                other = self.dry_run(assignment)
                self.assertEqual(other.returncode, 0, other.stderr)
                self.assertEqual(default.stdout, other.stdout)

    def test_full_disc_build_checks_sdk_coherence_after_programs(self) -> None:
        quake = run(
            "make",
            "-n",
            "--no-print-directory",
            "disc",
            "DIST=/tmp/psoxide-quake-contract",
            cwd=ROOT,
            check=False,
        )
        self.assertEqual(quake.returncode, 0, quake.stderr)
        programs_at = quake.stdout.index("PSOXIDE_FROM=")
        coherence_at = quake.stdout.index('expected="local:')
        stamp_at = quake.stdout.index("programs.psoxide-revision.tmp")
        verify_at = quake.stdout.index("tools/quake_disc.py verify")
        self.assertIn(f"DIST={ROOT}/games/voxide/dist", quake.stdout)
        self.assertIn(f"DIST={ROOT}/games/gh-psx/dist", quake.stdout)
        self.assertLess(programs_at, coherence_at)
        self.assertLess(coherence_at, stamp_at)
        self.assertLess(stamp_at, verify_at)

    def test_lock_audit_includes_both_psoxide_inputs(self) -> None:
        script = (ROOT / "tools" / "check-locks.sh").read_text(encoding="utf-8")
        self.assertIn("games/PSoXide ", script)
        self.assertIn("games/PSoXide-runtime ", script)

    def test_disc_runtime_crates_do_not_use_quakes_frozen_sdk(self) -> None:
        paths = (
            ROOT / "loader" / "Cargo.toml",
            ROOT / "carousel" / "Cargo.toml",
            ROOT / "launcher" / "Cargo.toml",
            ROOT / "tools" / "mkdisc" / "Cargo.toml",
        )
        for path in paths:
            manifest = path.read_text(encoding="utf-8")
            self.assertNotIn("games/PSoXide/", manifest, path)
            self.assertIn("games/PSoXide-runtime/", manifest, path)

        launcher = (ROOT / "launcher" / "src" / "main.rs").read_text(
            encoding="utf-8"
        )
        self.assertNotIn("games/PSoXide/assets/", launcher)
        self.assertIn("games/PSoXide-runtime/assets/", launcher)

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
                    f"PROGRAMS_PSOXIDE={fixture.psoxide}",
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

    def test_cortex_bakes_separate_current_and_legacy_projects(self) -> None:
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn(
            "$(CORTEX_CURRENT_PSOXIDE)/editor/projects/default", makefile
        )
        self.assertIn(
            "CORTEX_CURRENT_PROJECT := $(BUILD)/cortex-current", makefile
        )
        self.assertIn(
            "$(CORTEX_LEGACY_PSOXIDE)/editor/samples/cortex_v1", makefile
        )
        self.assertIn(
            "CORTEX_LEGACY_PROJECT := $(BUILD)/cortex-legacy", makefile
        )
        self.assertIn("b50e90266084beea85a94a7a194955dfe9bf6228", makefile)
        self.assertIn("687d2ae7681f9de3090dc89635beac99d1654c93", makefile)
        self.assertNotIn(
            "$(CORTEX_CURRENT_PSOXIDE)/editor/samples/cortex_v1", makefile
        )
        self.assertIn(
            "CORTEX_CURRENT_GUEST_STAGE_ROOT ?= "
            "/tmp/psoxide-psx-guest-v1-cortex-current",
            makefile,
        )
        self.assertIn(
            "CORTEX_LEGACY_GUEST_STAGE_ROOT ?= "
            "/tmp/psoxide-psx-guest-v1-cortex-legacy",
            makefile,
        )
        self.assertIn(
            'PSOXIDE_GUEST_STAGE_ROOT="$(CORTEX_CURRENT_GUEST_STAGE_ROOT)"',
            makefile,
        )
        self.assertIn(
            'PSOXIDE_GUEST_STAGE_ROOT="$(CORTEX_LEGACY_GUEST_STAGE_ROOT)"',
            makefile,
        )
        self.assertEqual(
            makefile.count(
                'PSOXIDE_GUEST_CARGO_HOME="$(CORTEX_GUEST_CARGO_HOME)"'
            ),
            2,
        )

    def test_half_life_pressing_is_the_default_disc_plus_half_life(self) -> None:
        default = self.dry_run()
        half_life = self.dry_run("HL=1")
        self.assertEqual(half_life.returncode, 0, half_life.stderr)
        self.assertIn('--image "HALF-LIFE=', half_life.stdout)
        self.assertNotIn('--image "HALF-LIFE=', default.stdout)
        for pressing in (default, half_life):
            self.assertIn('--image "CORTEX IGNITION=', pressing.stdout)
            self.assertIn('--image "CORTEX IGNITION LEGACY=', pressing.stdout)
        self.assertIn('--gate "CORTEX IGNITION"', default.stdout)
        self.assertIn('--gate "CORTEX IGNITION LEGACY"', default.stdout)
        self.assertNotIn('--gate "CORTEX IGNITION"', half_life.stdout)
        self.assertNotIn('--gate "CORTEX IGNITION LEGACY"', half_life.stdout)
        current_at = half_life.stdout.index('--image "CORTEX IGNITION=')
        legacy_at = half_life.stdout.index('--image "CORTEX IGNITION LEGACY=')
        half_life_at = half_life.stdout.index('--image "HALF-LIFE=')
        self.assertLess(current_at, legacy_at)
        self.assertLess(legacy_at, half_life_at)
        # Everything the default pressing carries, the HL pressing carries too.
        for argument in (
            '--image "QUAKE SHAREWARE=',
            f'--version-of "QUAKE SHAREWARE={self.pinned_menu_version()}"',
            "tools/quake_disc.py verify",
            "tools/quake_disc.py receipt",
        ):
            with self.subTest(argument=argument):
                self.assertIn(argument, default.stdout)
                self.assertIn(argument, half_life.stdout)
        self.assertIn("PSoXide Demo Disc HL.bin", half_life.stdout)

    def test_half_life_demo_build_packs_without_installing(self) -> None:
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn(
            "cargo run --release -- pack --psoxide $(PROGRAMS_PSOXIDE)",
            makefile,
        )
        self.assertNotIn(
            "cargo run --release -- disc --psoxide $(PROGRAMS_PSOXIDE)",
            makefile,
        )

    def test_distribution_targets_are_blocked_before_they_build_anything(self) -> None:
        blocked = run(
            "make", "--no-print-directory", "publication-block", cwd=ROOT, check=False
        )
        self.assertNotEqual(blocked.returncode, 0)
        self.assertIn("publication is blocked", blocked.stdout)
        self.assertIn("Quake 1.06 shareware data", blocked.stdout)

        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        for target in ("release-web", "itch"):
            with self.subTest(target=target):
                self.assertIn(f"\n{target}: publication-block\n", makefile)
                dry = run(
                    "make",
                    "-n",
                    "--no-print-directory",
                    target,
                    "DIST=/tmp/psoxide-quake-contract",
                    cwd=ROOT,
                    check=False,
                )
                # The block has to come out before the target's own first line,
                # which is as far as this can read: `make -n` recurses into the
                # $(MAKE) disc below it, and that needs the submodules.
                block_at = dry.stdout.index("publication is blocked")
                self.assertLess(block_at, dry.stdout.index('test -z "'))


class FailClosedDefaultTests(unittest.TestCase):
    """Every way a Quake payload can be wrong has to stop the default path."""

    def verify(
        self, fixture: QuakeFixture, **overrides: str
    ) -> subprocess.CompletedProcess[str]:
        assignments = {
            "PSOXIDE": str(fixture.psoxide),
            "PROGRAMS_PSOXIDE": str(fixture.psoxide),
            "PROGRAMS_EXPECTED_PSOXIDE_REV": fixture.psoxide_revision,
            "BUILD": str(fixture.programs_stamp.parent),
            "QUAKE_SRC": str(fixture.source),
            "QUAKE_EXPECTED_REV": fixture.revision,
            "QUAKE_EXPECTED_PSOXIDE_REV": fixture.psoxide_revision,
            "QUAKE_EXPECTED_PROVENANCE_SHA256": digest(fixture.provenance),
            "QUAKE_EXPECTED_CUE_SHA256": digest(fixture.cue),
            "QUAKE_EXPECTED_BIN_SHA256": digest(fixture.bin),
            "QUAKE_EXPECTED_EXE_SHA256": digest(fixture.exe),
        }
        assignments.update(overrides)
        return run(
            "make",
            "--no-print-directory",
            "quake-verify",
            *(f"{key}={value}" for key, value in assignments.items()),
            cwd=ROOT,
            check=False,
        )

    def test_a_correct_payload_passes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = QuakeFixture(Path(directory))
            passing = self.verify(fixture)
            self.assertEqual(passing.returncode, 0, passing.stdout + passing.stderr)

    def test_absent_stale_dirty_and_mispinned_payloads_all_fail(self) -> None:
        cases = (
            ("absent", {"QUAKE_SRC": "/nonexistent/quake"}, None, "does not exist"),
            (
                "stale quake pin",
                {"QUAKE_EXPECTED_REV": "0" * 40},
                None,
                "Quake source checkout revision mismatch",
            ),
            (
                "wrong psoxide pin",
                {"QUAKE_EXPECTED_PSOXIDE_REV": "0" * 40},
                None,
                "PSoXide checkout revision mismatch",
            ),
            (
                "stale artifact hash",
                {"QUAKE_EXPECTED_BIN_SHA256": "0" * 64},
                None,
                "bin SHA-256 mismatch",
            ),
            (
                "dirty quake checkout",
                {},
                lambda fixture: (fixture.source / "tracked.txt").write_text(
                    "changed\n", encoding="ascii"
                ),
                "Quake source checkout is dirty",
            ),
            (
                "dirty psoxide checkout",
                {},
                lambda fixture: (fixture.psoxide / "sdk.txt").write_text(
                    "changed\n", encoding="ascii"
                ),
                "PSoXide checkout is dirty",
            ),
            (
                "stale ordinary programs",
                {},
                lambda fixture: fixture.programs_stamp.write_text(
                    "0" * 40 + "\n", encoding="ascii"
                ),
                "ordinary-program SDK revision mismatch",
            ),
            (
                "missing program stamp",
                {},
                lambda fixture: fixture.programs_stamp.unlink(),
                "revision stamp does not exist",
            ),
        )
        for label, overrides, mutate, error in cases:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                fixture = QuakeFixture(Path(directory))
                if mutate is not None:
                    mutate(fixture)
                failed = self.verify(fixture, **overrides)
                self.assertNotEqual(failed.returncode, 0, failed.stdout)
                self.assertIn(error, failed.stderr)

    def test_repin_prints_the_pins_a_built_tree_implies(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = QuakeFixture(Path(directory))
            printed = run(
                "make",
                "--no-print-directory",
                "quake-repin",
                f"QUAKE_SRC={fixture.source}",
                cwd=ROOT,
                check=False,
            )
            self.assertEqual(printed.returncode, 0, printed.stderr)
            expected = {
                "QUAKE_EXPECTED_REV": fixture.revision,
                "QUAKE_EXPECTED_PSOXIDE_REV": fixture.psoxide_revision,
                "QUAKE_EXPECTED_PROVENANCE_SHA256": digest(fixture.provenance),
                "QUAKE_EXPECTED_CUE_SHA256": digest(fixture.cue),
                "QUAKE_EXPECTED_BIN_SHA256": digest(fixture.bin),
                "QUAKE_EXPECTED_EXE_SHA256": digest(fixture.exe),
            }
            for name, value in expected.items():
                with self.subTest(pin=name):
                    self.assertIn(f"{name} ?= {value}", printed.stdout)
            # Every pin the Makefile holds is a pin the repin prints.
            makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
            for name in re.findall(r"^(QUAKE_EXPECTED_\w+) \?=", makefile, re.MULTILINE):
                with self.subTest(pin=name):
                    self.assertIn(f"{name} ?= ", printed.stdout)
            self.assertNotIn("WARNING", printed.stdout)

    def test_repin_refuses_to_speak_for_a_dirty_tree(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = QuakeFixture(Path(directory))
            (fixture.source / "tracked.txt").write_text("changed\n", encoding="ascii")
            printed = run(
                "make",
                "--no-print-directory",
                "quake-repin",
                f"QUAKE_SRC={fixture.source}",
                cwd=ROOT,
                check=False,
            )
            self.assertEqual(printed.returncode, 0, printed.stderr)
            self.assertIn("WARNING: the Quake tree is dirty", printed.stdout)

    def test_the_ordinary_check_runs_the_quake_pin_check(self) -> None:
        checked = run(
            "make", "-n", "--no-print-directory", "check", cwd=ROOT, check=False
        )
        self.assertEqual(checked.returncode, 0, checked.stderr)
        self.assertIn("tools/quake_disc.py verify", checked.stdout)

    def test_layout_cannot_start_before_the_payload_is_verified(self) -> None:
        laid_out = run(
            "make",
            "-n",
            "--no-print-directory",
            "disc-only",
            "DIST=/tmp/psoxide-quake-contract",
            cwd=ROOT,
            check=False,
        )
        self.assertEqual(laid_out.returncode, 0, laid_out.stderr)
        stamp_at = laid_out.stdout.index("programs.psoxide-revision")
        verify_at = laid_out.stdout.index("tools/quake_disc.py verify")
        layout_at = laid_out.stdout.index('--image "QUAKE SHAREWARE=')
        receipt_at = laid_out.stdout.index("tools/quake_disc.py receipt")
        self.assertLess(stamp_at, verify_at)
        self.assertLess(verify_at, layout_at)
        self.assertLess(layout_at, receipt_at)


class HeadlessChainloadTests(unittest.TestCase):
    QUAKE_LBA = 50
    PAYLOAD_FNV = 0x1234_5678
    MENU_VERSION = "q2d26f9e"

    @classmethod
    def default_entries(cls) -> tuple[tuple[str, int, int], ...]:
        """The standard pressing's locked shape: two hidden, seven, Quake.

        Eight visible programs plus the launcher's CREDITS card is nine, and
        the headless route's two RIGHT presses still land on Quake.
        """
        cortex = (
            ("CORTEX IGNITION", 30, check_quake_headless.FLAG_HIDDEN),
            ("CORTEX IGNITION LEGACY", 31, check_quake_headless.FLAG_HIDDEN),
        )
        filler = tuple(
            (f"PROGRAM {index}", 32 + index, 0) for index in range(7)
        )
        return cortex + filler + ((check_quake_headless.QUAKE_ENTRY, cls.QUAKE_LBA, 0),)

    @classmethod
    def make_disc_image(
        cls, root: Path, entries: tuple[tuple[str, int, int], ...] | None = None
    ) -> Path:
        entries = cls.default_entries() if entries is None else entries
        image = bytearray((cls.QUAKE_LBA + 2) * check_quake_headless.SECTOR_BYTES)
        toc = bytearray(
            check_quake_headless.TOC_SECTORS
            * check_quake_headless.USER_DATA_BYTES
        )
        toc[:8] = check_quake_headless.TOC_MAGIC
        toc[8:12] = len(entries).to_bytes(4, "little")
        for index, (name, lba, flags) in enumerate(entries):
            at = (
                check_quake_headless.TOC_HEADER_BYTES
                + index * check_quake_headless.TOC_ENTRY_BYTES
            )
            encoded = name.encode("ascii")
            toc[at : at + len(encoded)] = encoded
            toc[
                at
                + check_quake_headless.TOC_NAME_BYTES : at
                + check_quake_headless.TOC_NAME_BYTES
                + 4
            ] = lba.to_bytes(4, "little")
            toc[
                at
                + check_quake_headless.TOC_FLAGS_AT : at
                + check_quake_headless.TOC_FLAGS_AT
                + 4
            ] = flags.to_bytes(4, "little")
            if name != check_quake_headless.QUAKE_ENTRY:
                continue
            toc[
                at
                + check_quake_headless.TOC_PAYLOAD_FNV_AT : at
                + check_quake_headless.TOC_PAYLOAD_FNV_AT
                + 4
            ] = cls.PAYLOAD_FNV.to_bytes(4, "little")
            version = cls.MENU_VERSION.encode("ascii")
            toc[
                at
                + check_quake_headless.TOC_VERSION_AT : at
                + check_quake_headless.TOC_VERSION_AT
                + len(version)
            ] = version
        for sector in range(check_quake_headless.TOC_SECTORS):
            source = sector * check_quake_headless.USER_DATA_BYTES
            target = (
                (check_quake_headless.TOC_LBA + sector)
                * check_quake_headless.SECTOR_BYTES
                + check_quake_headless.USER_DATA_AT
            )
            image[target : target + check_quake_headless.USER_DATA_BYTES] = toc[
                source : source + check_quake_headless.USER_DATA_BYTES
            ]

        header = bytearray(check_quake_headless.USER_DATA_BYTES)
        header[:8] = check_quake_headless.PSX_EXE_MAGIC
        header[0x10:0x14] = (0x8001_0000).to_bytes(4, "little")
        header[0x18:0x1C] = (0x8001_0000).to_bytes(4, "little")
        header[0x1C:0x20] = (4_096).to_bytes(4, "little")
        target = (
            cls.QUAKE_LBA * check_quake_headless.SECTOR_BYTES
            + check_quake_headless.USER_DATA_AT
        )
        image[target : target + check_quake_headless.USER_DATA_BYTES] = header
        path = root / "disc.bin"
        path.write_bytes(image)
        return path

    def receipt_output(self, **overrides: object) -> dict[str, object]:
        output = {
            "quake_toc": {
                "exe_lba": self.QUAKE_LBA,
                "payload_fnv1a32": f"0x{self.PAYLOAD_FNV:08x}",
                "menu_version": self.MENU_VERSION,
            },
            "embedded_quake_data_sectors": 9_465,
            "embedded_quake_matches_input_except_msf": True,
        }
        output.update(overrides)
        return output

    def test_route_selects_visible_quake_on_the_default_carousel(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            image = self.make_disc_image(Path(directory))
            selected, menu, payload = check_quake_headless.quake_menu_entry(
                image, self.QUAKE_LBA
            )
            self.assertEqual(len(menu), check_quake_headless.DEFAULT_MENU_ENTRIES)
            self.assertEqual(
                selected + 1, check_quake_headless.DEFAULT_MENU_ENTRIES - 1
            )
            self.assertEqual(menu[selected], check_quake_headless.QUAKE_ENTRY)
            self.assertEqual(menu[0], "PROGRAM 0")
            self.assertNotIn("CORTEX IGNITION", menu)
            self.assertNotIn("CORTEX IGNITION LEGACY", menu)
            self.assertEqual(menu[-1], "CREDITS")
            self.assertEqual(
                payload,
                {
                    "exe_lba": self.QUAKE_LBA,
                    "payload_fnv1a32": f"0x{self.PAYLOAD_FNV:08x}",
                    "menu_version": self.MENU_VERSION,
                },
            )
            self.assertEqual(
                check_quake_headless.embedded_exe_evidence(image, self.QUAKE_LBA),
                (0x8001_0000, 0x8001_0000, 4_096),
            )

    def test_route_selects_visible_quake_on_the_half_life_carousel(self) -> None:
        default = self.default_entries()
        entries = (
            tuple((name, lba, 0) for name, lba, _ in default[:2])
            + (("HALF-LIFE", 41, 0),)
            + default[2:]
        )
        with tempfile.TemporaryDirectory() as directory:
            image = self.make_disc_image(Path(directory), entries)
            selected, menu, _ = check_quake_headless.quake_menu_entry(
                image, self.QUAKE_LBA, 12
            )
            self.assertEqual(len(menu), 12)
            self.assertEqual(selected + 1, 11)
            self.assertEqual(menu[:3], [
                "CORTEX IGNITION",
                "CORTEX IGNITION LEGACY",
                "HALF-LIFE",
            ])
            self.assertEqual(menu[selected], check_quake_headless.QUAKE_ENTRY)
            self.assertEqual(menu[-1], "CREDITS")

    def test_a_disc_without_a_visible_quake_entry_fails(self) -> None:
        cases = (
            (
                "absent",
                tuple(
                    entry
                    for entry in self.default_entries()
                    if entry[0] != check_quake_headless.QUAKE_ENTRY
                )
                + (("TENTH", 41, 0),),
                "no QUAKE SHAREWARE entry",
            ),
            (
                "hidden",
                tuple(
                    (name, lba, check_quake_headless.FLAG_HIDDEN)
                    if name == check_quake_headless.QUAKE_ENTRY
                    else (name, lba, flags)
                    for name, lba, flags in self.default_entries()
                ),
                "hidden from the carousel",
            ),
            (
                "wrong entry count",
                self.default_entries() + (("EXTRA", 41, 0),),
                "visible entries, expected",
            ),
        )
        for label, entries, error in cases:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                image = self.make_disc_image(Path(directory), entries)
                with self.assertRaisesRegex(check_quake_headless.CheckError, error):
                    check_quake_headless.quake_menu_entry(image, self.QUAKE_LBA)

    def test_payload_identity_is_held_against_the_receipt(self) -> None:
        payload = {
            "exe_lba": self.QUAKE_LBA,
            "payload_fnv1a32": f"0x{self.PAYLOAD_FNV:08x}",
            "menu_version": self.MENU_VERSION,
        }
        check_quake_headless.require_payload_identity(payload, self.receipt_output())
        cases = (
            ({"exe_lba": 41}, "exe_lba"),
            ({"payload_fnv1a32": "0xdeadbeef"}, "payload_fnv1a32"),
            ({"menu_version": "q0000000"}, "menu_version"),
        )
        for drift, error in cases:
            with self.subTest(field=error):
                toc = dict(self.receipt_output()["quake_toc"])
                toc.update(drift)
                with self.assertRaisesRegex(check_quake_headless.CheckError, error):
                    check_quake_headless.require_payload_identity(
                        payload, self.receipt_output(quake_toc=toc)
                    )
        with self.assertRaisesRegex(check_quake_headless.CheckError, "embedded Quake"):
            check_quake_headless.require_payload_identity(
                payload,
                self.receipt_output(embedded_quake_matches_input_except_msf=False),
            )

    def test_replays_must_agree_on_everything_they_observed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            logs = {}
            for kind in check_quake_headless.LOG_KINDS:
                path = root / f"{kind}.csv"
                path.write_text(f"{kind}\n", encoding="ascii")
                logs[kind] = path
            base = {"stdout_core": "same", **logs}
            for field in check_quake_headless.DETERMINISTIC_FIELDS:
                base[field] = 1
            check_quake_headless.require_identical_replays(base, dict(base))

            for field in check_quake_headless.DETERMINISTIC_FIELDS:
                with self.subTest(field=field):
                    drifted = dict(base)
                    drifted[field] = 2
                    with self.assertRaisesRegex(
                        check_quake_headless.CheckError, field
                    ):
                        check_quake_headless.require_identical_replays(base, drifted)

            other = dict(base)
            other["stdout_core"] = "different"
            with self.assertRaisesRegex(check_quake_headless.CheckError, "stdout"):
                check_quake_headless.require_identical_replays(base, other)

            second = root / "second-route.csv"
            second.write_text("elsewhere\n", encoding="ascii")
            drifted_log = dict(base)
            drifted_log["route"] = second
            with self.assertRaisesRegex(check_quake_headless.CheckError, "route"):
                check_quake_headless.require_identical_replays(base, drifted_log)

    def test_only_build_independent_values_are_pinned(self) -> None:
        source = (ROOT / "tools" / "check_quake_headless.py").read_text(
            encoding="ascii"
        )
        # These moved with the launcher binary, which changes on every commit
        # here, so they are held to run-to-run equality and nothing more.
        for pin in (
            "EXPECTED_CYCLES",
            "EXPECTED_PC ",
            "EXPECTED_ROUTE_TICKS",
            "EXPECTED_PAD_POLLS",
            "EXPECTED_CD_COMMANDS",
            "EXPECTED_LOG_SHA256",
        ):
            self.assertNotIn(pin, source)
        for field in ("cycles", "route_ticks", "pad_polls"):
            self.assertIn(field, check_quake_headless.DETERMINISTIC_FIELDS)

    def test_each_pressing_has_its_own_visible_frame_pins(self) -> None:
        self.assertEqual(
            set(check_quake_headless.EXPECTED_FRAME_FNV_BY_MENU_ENTRIES),
            {check_quake_headless.DEFAULT_MENU_ENTRIES, 12},
        )
        for menu_entries, (vram, display) in (
            check_quake_headless.EXPECTED_FRAME_FNV_BY_MENU_ENTRIES.items()
        ):
            with self.subTest(menu_entries=menu_entries):
                result = {
                    "tick": check_quake_headless.EXPECTED_TICK,
                    "vram_fnv": vram,
                    "display_fnv": display,
                    "display_width": check_quake_headless.EXPECTED_DISPLAY[0],
                    "display_height": check_quake_headless.EXPECTED_DISPLAY[1],
                }
                check_quake_headless.require_pins(
                    result, "fixture", menu_entries
                )

        with self.assertRaisesRegex(check_quake_headless.CheckError, "no frame pins"):
            check_quake_headless.require_pins(result, "fixture", 13)

    def test_cd_evidence_requires_header_and_payload_read_sequences(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "cd.csv"
            log.write_text(
                "cycle,command,param_len,params\n"
                "1,0x02,3,00 02 40\n"
                "2,0x15,0,\n"
                "3,0x06,0,\n"
                "4,0x02,3,00 02 41\n"
                "5,0x15,0,\n"
                "6,0x06,0,\n"
                "7,0x02,3,00 02 45\n"
                "8,0x15,0,\n"
                "9,0x06,0,\n",
                encoding="ascii",
            )
            self.assertEqual(
                check_quake_headless.cd_evidence(log, 40, 4_096, 20, 100),
                (9, 2, 45),
            )
            log.write_text(
                "cycle,command,param_len,params\n"
                "1,0x02,3,00 02 40\n"
                "2,0x15,0,\n"
                "3,0x06,0,\n",
                encoding="ascii",
            )
            with self.assertRaises(check_quake_headless.CheckError):
                check_quake_headless.cd_evidence(log, 40, 4_096, 20, 100)

    def test_runtime_markers_are_ordered_and_fail_closed(self) -> None:
        output = "\n".join(check_quake_headless.MARKERS)
        check_quake_headless.require_runtime_markers(output)
        with self.assertRaises(check_quake_headless.CheckError):
            check_quake_headless.require_runtime_markers(
                "\n".join(reversed(check_quake_headless.MARKERS))
            )
        with self.assertRaises(check_quake_headless.CheckError):
            check_quake_headless.require_runtime_markers(
                output + "\nquake-psx: Rust initial level load failed"
            )

    def test_headless_gate_cannot_create_media_dumps(self) -> None:
        source = (ROOT / "tools" / "check_quake_headless.py").read_text(
            encoding="ascii"
        )
        self.assertIn('"--embedded-playtest"', source)
        for flag in ("--dump-display", "--dump-vram", "--dump-hw", "--dump-audio"):
            self.assertNotIn(flag, source)


if __name__ == "__main__":
    unittest.main()
