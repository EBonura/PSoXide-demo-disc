//! A synthetic Quake build tree for the tests: real git repositories, a cue,
//! bin and exe, and the provenance sidecar that vouches for them. Every
//! constant here is written out on purpose instead of imported from the
//! verifier, so a drifted constant in the verifier fails a test rather than
//! agreeing with itself.
//!
//! This file stands alone (no `crate::` paths) because the integration tests
//! in `tests/` include it directly, to drive the Makefile with the same tree.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const SECTOR_BYTES: usize = 2352;
pub const USER_DATA_AT: usize = 24;
pub const USER_DATA_BYTES: usize = 2048;
pub const BOOT_EXE_LBA: usize = 22;
pub const TOC_LBA: usize = 22;
pub const TOC_SECTORS: usize = 4;
pub const TOC_HEADER_BYTES: usize = 0x16C;
pub const TOC_NAME_BYTES: usize = 24;
pub const TOC_DESC_BYTES: usize = 224;
pub const PSOXIDE_REV_FILE: &str = "host/quake-build/main.rs";
pub const SHAREWARE_PAK_SHA256: &str =
    "35a9c55e5e5a284a159ad2a62e0e8def23d829561fe2f54eb402dbc0a9a946af";
pub const SHAREWARE_PAK_BYTES: u64 = 18_689_235;

/// Run a command, require success, return trimmed stdout.
pub fn run(cwd: &Path, program: &str, args: &[&str]) -> String {
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("command starts");
    assert!(
        output.status.success(),
        "{program} {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub fn git(cwd: &Path, args: &[&str]) -> String {
    run(cwd, "git", args)
}

pub fn digest(path: &Path) -> String {
    let bytes = fs::read(path).expect("file to hash");
    Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A fresh repository with one author and no signing, so a commit never
/// waits on the host's git configuration.
fn init_repo(path: &Path) {
    fs::create_dir(path).expect("repo dir");
    git(path, &["init", "-q"]);
    git(path, &["config", "user.name", "Test"]);
    git(path, &["config", "user.email", "test@example.invalid"]);
    git(path, &["config", "commit.gpgsign", "false"]);
}

fn head(path: &Path) -> String {
    git(path, &["rev-parse", "HEAD"])
}

fn write_json(path: &Path, value: &Value) {
    fs::write(
        path,
        serde_json::to_string_pretty(value).expect("json") + "\n",
    )
    .expect("write json");
}

pub struct QuakeFixture {
    _guard: tempfile::TempDir,
    pub root: PathBuf,
    pub psoxide: PathBuf,
    pub programs_stamp: PathBuf,
    pub source: PathBuf,
    pub cue: PathBuf,
    pub bin: PathBuf,
    pub exe: PathBuf,
    pub provenance: PathBuf,
    pub components: Value,
    pub psoxide_revision: String,
    pub revision: String,
    pub provenance_sha256: String,
}

impl QuakeFixture {
    pub fn new() -> QuakeFixture {
        let guard = tempfile::tempdir().expect("temp dir");
        // Resolved, because the verifier compares canonical paths and macOS
        // keeps its temp dir behind a symlink.
        let root = guard.path().canonicalize().expect("canonical temp dir");

        let psoxide = root.join("psoxide");
        init_repo(&psoxide);
        fs::write(psoxide.join("sdk.txt"), "pinned\n").unwrap();
        let mut components = json!({
            "sdk": {"repository": "EBonura/PSoXide", "revision": "a".repeat(40), "paths": ["sdk"]},
            "emulator": {"repository": "EBonura/PSoXide-emulator", "revision": "b".repeat(40), "paths": ["emu"]},
        });
        write_json(
            &psoxide.join("components.lock.json"),
            &json!({"schema": 1, "components": components}),
        );
        git(&psoxide, &["add", "sdk.txt", "components.lock.json"]);
        git(&psoxide, &["commit", "-q", "-m", "fixture"]);
        let psoxide_revision = head(&psoxide);
        components["editor"] = json!({
            "repository": "EBonura/PSoXide-editor", "revision": psoxide_revision, "paths": ["engine"],
        });
        let programs_stamp = root.join("programs.psoxide-revision");
        fs::write(&programs_stamp, format!("{psoxide_revision}\n")).unwrap();

        let source = root.join("quake");
        init_repo(&source);
        fs::write(source.join(".gitignore"), "dist/\n").unwrap();
        fs::write(source.join("tracked.txt"), "pinned\n").unwrap();
        write_json(
            &source.join("components.lock.json"),
            &json!({"schema": 1, "components": components}),
        );
        let declaration = source.join(PSOXIDE_REV_FILE);
        fs::create_dir_all(declaration.parent().unwrap()).unwrap();
        fs::write(
            &declaration,
            format!("const PSOXIDE_REV: &str = \"{psoxide_revision}\";\n"),
        )
        .unwrap();
        git(
            &source,
            &[
                "add",
                ".gitignore",
                "tracked.txt",
                "components.lock.json",
                PSOXIDE_REV_FILE,
            ],
        );
        git(&source, &["commit", "-q", "-m", "fixture"]);
        let revision = head(&source);

        let dist = source.join("dist");
        fs::create_dir(&dist).unwrap();
        let (cue, bin, exe) = (
            dist.join("quake-psx.cue"),
            dist.join("quake-psx.bin"),
            dist.join("quake-psx.exe"),
        );
        let mut image = vec![0u8; 24 * SECTOR_BYTES];
        let boot_at = BOOT_EXE_LBA * SECTOR_BYTES + USER_DATA_AT;
        image[boot_at..boot_at + 8].copy_from_slice(b"PS-X EXE");
        fs::write(&bin, image).unwrap();
        fs::write(
            &cue,
            "FILE \"quake-psx.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n",
        )
        .unwrap();
        fs::write(&exe, b"PS-X EXE\0fixture").unwrap();

        let mut fixture = QuakeFixture {
            _guard: guard,
            root,
            psoxide,
            programs_stamp,
            source,
            provenance: dist.join("quake-psx.provenance.json"),
            cue,
            bin,
            exe,
            components,
            psoxide_revision,
            revision,
            provenance_sha256: String::new(),
        };
        fixture.write_provenance(&fixture.provenance_document());
        fixture.provenance_sha256 = digest(&fixture.provenance);
        fixture
    }

    fn artifact(path: &Path) -> Value {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        json!({"file": name, "sha256": digest(path), "bytes": fs::metadata(path).unwrap().len()})
    }

    pub fn provenance_document(&self) -> Value {
        json!({
            "schema": 1,
            "quake_source": {"revision": self.revision, "tree_clean": true},
            "psoxide": {
                "repository": "EBonura/PSoXide-editor",
                "components": self.components,
                "revision": self.psoxide_revision,
                "tree_clean": true,
                "source_kind": "local_checkout",
            },
            "shareware": {"pak0_sha256": SHAREWARE_PAK_SHA256, "pak0_bytes": SHAREWARE_PAK_BYTES},
            "build": {
                "guest_stage_schema": 1,
                "guest_recipe_sha256": "1".repeat(64),
                "rust_toolchain_sha256": "2".repeat(64),
                "rustc_version": "rustc fixture\nrelease: fixture",
                "cargo_version": "cargo fixture\nrelease: fixture",
                "profile": "release",
                "features": [],
            },
            "artifacts": {
                "cue": Self::artifact(&self.cue),
                "bin": Self::artifact(&self.bin),
                "exe": Self::artifact(&self.exe),
            },
        })
    }

    pub fn write_provenance(&self, document: &Value) {
        write_json(&self.provenance, document);
    }

    /// Rewrite the sidecar and re-pin its hash, as a repin would.
    pub fn write_and_pin_provenance(&mut self, document: &Value) {
        self.write_provenance(document);
        self.provenance_sha256 = digest(&self.provenance);
    }

    pub fn commit_quake_declaration(&mut self, declaration: &str) {
        fs::write(self.source.join(PSOXIDE_REV_FILE), declaration).unwrap();
        git(&self.source, &["add", PSOXIDE_REV_FILE]);
        git(&self.source, &["commit", "-q", "-m", "change declaration"]);
        self.revision = head(&self.source);
    }

    pub fn advance_psoxide(&mut self) {
        fs::write(self.psoxide.join("sdk.txt"), "next\n").unwrap();
        git(&self.psoxide, &["add", "sdk.txt"]);
        git(&self.psoxide, &["commit", "-q", "-m", "advance fixture"]);
        self.psoxide_revision = head(&self.psoxide);
    }
}

impl QuakeFixture {
    /// A combined disc that carries the fixture's Quake bin at LBA 30, with a
    /// one-row demo table naming it, and a cue for it.
    pub fn make_demo(&self) -> (PathBuf, PathBuf) {
        let image_lba = 30;
        let input = fs::read(&self.bin).unwrap();
        let mut image = vec![0u8; image_lba * SECTOR_BYTES];
        image.extend_from_slice(&input);

        let mut toc = vec![0u8; TOC_SECTORS * USER_DATA_BYTES];
        toc[..8].copy_from_slice(b"PSXDEMO4");
        toc[8..12].copy_from_slice(&1u32.to_le_bytes());
        let at = TOC_HEADER_BYTES;
        toc[at..at + 15].copy_from_slice(b"QUAKE SHAREWARE");
        let numbers = at + TOC_NAME_BYTES;
        let le = |value: usize| (value as u32).to_le_bytes();
        toc[numbers..numbers + 4].copy_from_slice(&le(image_lba + BOOT_EXE_LBA));
        toc[numbers + 4..numbers + 8].copy_from_slice(&le(image_lba));
        toc[numbers + 8..numbers + 12].copy_from_slice(&le(4));
        toc[numbers + 12..numbers + 16].copy_from_slice(&le(0x1234_5678));
        let description_at = numbers + 16;
        let description = b"Pinned Quake shareware Episode 1";
        toc[description_at..description_at + description.len()].copy_from_slice(description);
        let version_at = description_at + 2 * TOC_DESC_BYTES;
        let version = format!("q{}", &self.revision[..7]);
        toc[version_at..version_at + version.len()].copy_from_slice(version.as_bytes());
        for sector in 0..TOC_SECTORS {
            let from = sector * USER_DATA_BYTES;
            let to = (TOC_LBA + sector) * SECTOR_BYTES + USER_DATA_AT;
            image[to..to + USER_DATA_BYTES].copy_from_slice(&toc[from..from + USER_DATA_BYTES]);
        }

        let demo_bin = self.root.join("demo.bin");
        let demo_cue = self.root.join("demo.cue");
        fs::write(&demo_bin, image).unwrap();
        fs::write(
            &demo_cue,
            "FILE \"demo.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n",
        )
        .unwrap();
        (demo_cue, demo_bin)
    }
}
