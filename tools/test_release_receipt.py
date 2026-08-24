from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("release_receipt.py")
SPEC = importlib.util.spec_from_file_location("release_receipt", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
receipt = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = receipt
SPEC.loader.exec_module(receipt)


def run(*args: str, cwd: Path) -> str:
    result = subprocess.run(
        list(args), cwd=cwd, text=True, stdout=subprocess.PIPE, check=True
    )
    return result.stdout.strip()


def put_user_sector(image: bytearray, lba: int, data: bytes) -> None:
    at = lba * receipt.SECTOR_BYTES + receipt.USER_DATA_AT
    image[at : at + len(data)] = data


class ReleaseFixture:
    def __init__(self, root: Path) -> None:
        self.source = root / "source"
        self.source.mkdir()
        run("git", "init", "-q", cwd=self.source)
        run("git", "config", "user.name", "Fixture", cwd=self.source)
        run("git", "config", "user.email", "fixture@example.invalid", cwd=self.source)
        (self.source / "tracked.txt").write_text("release\n", encoding="ascii")
        (self.source / ".gitignore").write_text("data/\n.psoxide/\n", encoding="ascii")
        run("git", "add", "tracked.txt", ".gitignore", cwd=self.source)
        run("git", "commit", "-q", "-m", "fixture", cwd=self.source)
        revision = run("git", "rev-parse", "HEAD", cwd=self.source)
        psoxide = self.source / ".psoxide"
        psoxide.mkdir()
        (psoxide / ".psoxide-source").write_text(
            f"git:{revision}\n", encoding="ascii"
        )
        (psoxide / "sdk.rs").write_text("sdk\n", encoding="ascii")
        data = self.source / "data"
        data.mkdir()
        (data / "chunk.bin").write_bytes(b"fresh cooked data")
        self.cook_manifest = data / ".hlpsx-cook.json"
        self.cook_manifest.write_text(
            json.dumps(
                {
                    "schema": receipt.HL_COOK_SCHEMA,
                    "hl_psx_revision": revision,
                    "hl_psx_tree_sha256": receipt.source_tree_sha256(self.source),
                    "psoxide_source": f"git:{revision}",
                    "psoxide_revision": revision,
                    "psoxide_tree_sha256": receipt.psoxide_tree_sha256(psoxide),
                    "half_life_input_sha256": "a" * 64,
                    "cooked_tree_sha256": receipt.cooked_tree_sha256(self.source),
                },
                indent=2,
                sort_keys=True,
            )
            + "\n",
            encoding="ascii",
        )

        self.frontend = root / "frontend"
        self.frontend.write_bytes(b"frontend")
        self.programs: dict[str, Path] = {}
        inputs: dict[str, bytes] = {}
        payloads: dict[str, bytes] = {}
        for index, name in enumerate(receipt.REQUIRED_PROGRAMS):
            payload = bytes([index + 1]) * receipt.USER_DATA_BYTES
            image = bytearray(4 * receipt.SECTOR_BYTES)
            header = bytearray(receipt.USER_DATA_BYTES)
            header[:8] = receipt.PSX_EXE_MAGIC
            header[0x10:0x14] = (0x80010000).to_bytes(4, "little")
            header[0x18:0x1C] = (0x80010000).to_bytes(4, "little")
            header[0x1C:0x20] = len(payload).to_bytes(4, "little")
            put_user_sector(image, 1, header)
            put_user_sector(image, 2, payload)
            bin_path = root / f"input-{index}.bin"
            cue_path = root / f"input-{index}.cue"
            bin_path.write_bytes(image)
            cue_path.write_text(
                f'FILE "{bin_path.name}" BINARY\n'
                "  TRACK 01 MODE2/2352\n"
                "    INDEX 01 00:00:00\n",
                encoding="ascii",
            )
            self.programs[name] = cue_path
            inputs[name] = bytes(image)
            payloads[name] = payload

        image_lbas = {name: 30 + index * 4 for index, name in enumerate(receipt.REQUIRED_PROGRAMS)}
        combined = bytearray(50 * receipt.SECTOR_BYTES)
        toc = bytearray(receipt.TOC_SECTORS * receipt.USER_DATA_BYTES)
        toc[:8] = receipt.TOC_MAGIC
        toc[8:12] = len(receipt.REQUIRED_PROGRAMS).to_bytes(4, "little")
        for index, name in enumerate(receipt.REQUIRED_PROGRAMS):
            image_lba = image_lbas[name]
            combined[
                image_lba * receipt.SECTOR_BYTES : (image_lba + 4) * receipt.SECTOR_BYTES
            ] = inputs[name]
            at = receipt.TOC_HEADER_BYTES + index * receipt.TOC_ENTRY_BYTES
            encoded = name.encode("ascii")
            toc[at : at + len(encoded)] = encoded
            toc[at + 24 : at + 28] = (image_lba + 1).to_bytes(4, "little")
            toc[at + 28 : at + 32] = image_lba.to_bytes(4, "little")
            toc[at + 36 : at + 40] = receipt.fnv1a32(payloads[name]).to_bytes(
                4, "little"
            )
            version = f"v{index}".encode("ascii")
            toc[
                at + receipt.TOC_VERSION_AT : at + receipt.TOC_VERSION_AT + len(version)
            ] = version
        for offset in range(receipt.TOC_SECTORS):
            put_user_sector(
                combined,
                receipt.TOC_LBA + offset,
                toc[
                    offset * receipt.USER_DATA_BYTES : (offset + 1)
                    * receipt.USER_DATA_BYTES
                ],
            )
        self.combined_bin = root / "combined.bin"
        self.combined_cue = root / "combined.cue"
        self.combined_bin.write_bytes(combined)
        self.combined_cue.write_text(
            'FILE "combined.bin" BINARY\n'
            "  TRACK 01 MODE2/2352\n"
            "    INDEX 01 00:00:00\n",
            encoding="ascii",
        )

    def document(self) -> dict[str, object]:
        return receipt.build_document(
            self.combined_cue,
            self.frontend,
            "make disc HL=1 DIST=dist/hardware-candidate",
            self.programs,
            {name: self.source for name in receipt.REQUIRED_PROGRAMS},
        )


class ReleaseReceiptTests(unittest.TestCase):
    def test_records_and_verifies_all_program_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = ReleaseFixture(Path(directory))
            document = fixture.document()
            self.assertEqual(set(document["programs"]), set(receipt.REQUIRED_PROGRAMS))
            for row in document["programs"].values():
                self.assertEqual(row["source"]["tree_clean"], True)
                self.assertEqual(row["embedded"]["image_sectors"], 4)
                self.assertEqual(row["input"]["payload"]["bytes"], 2048)
            cooked = document["programs"]["HALF-LIFE"]["cooked_assets"]
            self.assertTrue(cooked["verified"])
            self.assertEqual(cooked["document"]["schema"], receipt.HL_COOK_SCHEMA)

    def test_half_life_cooked_asset_tamper_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = ReleaseFixture(Path(directory))
            (fixture.source / "data/chunk.bin").write_bytes(b"stale mutation")
            with self.assertRaisesRegex(
                receipt.ReceiptError, "does not match the current cooked assets"
            ):
                fixture.document()

    def test_half_life_manifest_revision_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = ReleaseFixture(Path(directory))
            document = json.loads(fixture.cook_manifest.read_text(encoding="ascii"))
            document["hl_psx_revision"] = "0" * 40
            fixture.cook_manifest.write_text(json.dumps(document), encoding="ascii")
            with self.assertRaisesRegex(
                receipt.ReceiptError, "revision does not match"
            ):
                fixture.document()

    def test_dirty_source_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = ReleaseFixture(Path(directory))
            (fixture.source / "untracked.txt").write_text("dirty\n", encoding="ascii")
            with self.assertRaisesRegex(receipt.ReceiptError, "source is dirty"):
                fixture.document()

    def test_embedded_tamper_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = ReleaseFixture(Path(directory))
            image = bytearray(fixture.combined_bin.read_bytes())
            image[30 * receipt.SECTOR_BYTES + 100] ^= 1
            fixture.combined_bin.write_bytes(image)
            with self.assertRaisesRegex(receipt.ReceiptError, "embedded sector"):
                fixture.document()

    def test_audio_tracks_are_hashed_but_only_data_track_is_embedded(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = ReleaseFixture(Path(directory))
            name = receipt.REQUIRED_PROGRAMS[0]
            cue = fixture.programs[name]
            image = receipt.image_for_cue(cue)
            with image.open("ab") as stream:
                stream.write(bytes([0xA5]) * (2 * receipt.SECTOR_BYTES))
            cue.write_text(
                f'FILE "{image.name}" BINARY\n'
                "  TRACK 01 MODE2/2352\n"
                "    INDEX 01 00:00:00\n"
                "  TRACK 02 AUDIO\n"
                "    INDEX 00 00:00:04\n"
                "    INDEX 01 00:02:04\n",
                encoding="ascii",
            )
            row = fixture.document()["programs"][name]
            self.assertEqual(row["input"]["bin_total_sectors"], 6)
            self.assertEqual(row["input"]["data_track_sectors"], 4)
            self.assertEqual(row["embedded"]["image_sectors"], 4)

    def test_sealed_verification_survives_deleted_build_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = ReleaseFixture(root)
            document = fixture.document()
            receipt_path = root / "combined.release-receipt.json"
            receipt_path.write_text(json.dumps(document), encoding="ascii")
            fixture.frontend.unlink()
            for cue in fixture.programs.values():
                receipt.image_for_cue(cue).unlink()
                cue.unlink()
            receipt.verify_sealed_document(receipt_path, document)

    def test_sealed_verification_rejects_combined_tamper(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = ReleaseFixture(root)
            document = fixture.document()
            receipt_path = root / "combined.release-receipt.json"
            receipt_path.write_text(json.dumps(document), encoding="ascii")
            image = bytearray(fixture.combined_bin.read_bytes())
            image[-1] ^= 1
            fixture.combined_bin.write_bytes(image)
            with self.assertRaisesRegex(receipt.ReceiptError, "SHA-256"):
                receipt.verify_sealed_document(receipt_path, document)


if __name__ == "__main__":
    unittest.main()
