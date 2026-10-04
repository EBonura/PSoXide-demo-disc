//! Fixtures build a tiny but real pressing in a temp directory: git source
//! checkouts, per-program input images and a combined image with a table.

use super::*;
use std::process::Command;

pub(crate) fn run_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn put_user_sector(image: &mut [u8], lba: usize, data: &[u8]) {
    let at = lba * SECTOR_BYTES as usize + disc::USER_DATA_AT as usize;
    image[at..at + data.len()].copy_from_slice(data);
}

const HL_PRESSING: [&str; 3] = ["CORTEX IGNITION", "QUAKE SHAREWARE", "HALF-LIFE"];

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    names: Vec<String>,
    source: PathBuf,
    cook_manifest: PathBuf,
    frontend: PathBuf,
    programs: Vec<(String, PathBuf)>,
    combined_bin: PathBuf,
    combined_cue: PathBuf,
}

impl Fixture {
    fn new(names: &[&str]) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = resolve(dir.path()).unwrap();
        let source = root.join("source");
        fs::create_dir(&source).unwrap();
        run_git(&source, &["init", "-q"]);
        run_git(&source, &["config", "user.name", "Fixture"]);
        run_git(
            &source,
            &["config", "user.email", "fixture@example.invalid"],
        );
        fs::write(source.join("tracked.txt"), "release\n").unwrap();
        fs::write(source.join(".gitignore"), "data/\n.psoxide/\n").unwrap();
        run_git(&source, &["add", "tracked.txt", ".gitignore"]);
        run_git(&source, &["commit", "-q", "-m", "fixture"]);
        let revision = run_git(&source, &["rev-parse", "HEAD"]);
        let psoxide = source.join(".psoxide");
        fs::create_dir(&psoxide).unwrap();
        fs::write(psoxide.join(".psoxide-source"), format!("git:{revision}\n")).unwrap();
        fs::write(psoxide.join("sdk.rs"), "sdk\n").unwrap();
        let data = source.join("data");
        fs::create_dir(&data).unwrap();
        fs::write(data.join("chunk.bin"), b"fresh cooked data").unwrap();
        let cook_manifest = data.join(".hlpsx-cook.json");
        let manifest = json!({
            "schema": HL_COOK_SCHEMA,
            "hl_psx_revision": revision,
            "hl_psx_tree_sha256": source_tree_sha256(&source).unwrap(),
            "psoxide_source": format!("git:{revision}"),
            "psoxide_revision": revision,
            "psoxide_tree_sha256": psoxide_tree_sha256(&psoxide).unwrap(),
            "half_life_input_sha256": "a".repeat(64),
            "cooked_tree_sha256": cooked_tree_sha256(&source).unwrap(),
        });
        fs::write(
            &cook_manifest,
            crate::util::dumps(&manifest, 2, true) + "\n",
        )
        .unwrap();
        let frontend = root.join("frontend");
        fs::write(&frontend, b"frontend").unwrap();

        let mut programs = Vec::new();
        let mut inputs = Vec::new();
        let mut payloads = Vec::new();
        for (index, name) in names.iter().enumerate() {
            let payload = vec![index as u8 + 1; USER_DATA_BYTES];
            let mut image = vec![0u8; 4 * SECTOR_BYTES as usize];
            let mut header = vec![0u8; USER_DATA_BYTES];
            header[..8].copy_from_slice(disc::PSX_EXE_MAGIC);
            header[0x10..0x14].copy_from_slice(&0x8001_0000u32.to_le_bytes());
            header[0x18..0x1C].copy_from_slice(&0x8001_0000u32.to_le_bytes());
            header[0x1C..0x20].copy_from_slice(&(payload.len() as u32).to_le_bytes());
            put_user_sector(&mut image, 1, &header);
            put_user_sector(&mut image, 2, &payload);
            let bin = root.join(format!("input-{index}.bin"));
            let cue = root.join(format!("input-{index}.cue"));
            fs::write(&bin, &image).unwrap();
            fs::write(
                &cue,
                format!(
                    "FILE \"{}\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n",
                    bin.file_name().unwrap().to_string_lossy()
                ),
            )
            .unwrap();
            programs.push((name.to_string(), cue));
            inputs.push(image);
            payloads.push(payload);
        }
        let image_lbas: Vec<usize> = (0..names.len()).map(|i| 30 + i * 4).collect();
        let mut combined = vec![0u8; (34 + 4 * names.len()) * SECTOR_BYTES as usize];
        let mut toc = vec![0u8; disc::TOC_SECTORS as usize * USER_DATA_BYTES];
        toc[..8].copy_from_slice(disc::TOC_MAGIC);
        toc[8..12].copy_from_slice(&(names.len() as u32).to_le_bytes());
        for (index, name) in names.iter().enumerate() {
            let lba = image_lbas[index];
            combined[lba * SECTOR_BYTES as usize..(lba + 4) * SECTOR_BYTES as usize]
                .copy_from_slice(&inputs[index]);
            let at = disc::TOC_HEADER_BYTES + index * disc::TOC_ENTRY_BYTES;
            toc[at..at + name.len()].copy_from_slice(name.as_bytes());
            toc[at + 24..at + 28].copy_from_slice(&(lba as u32 + 1).to_le_bytes());
            toc[at + 28..at + 32].copy_from_slice(&(lba as u32).to_le_bytes());
            toc[at + 36..at + 40].copy_from_slice(&fnv1a32(&payloads[index]).to_le_bytes());
            let version = format!("v{index}");
            let v = at + disc::TOC_VERSION_AT;
            toc[v..v + version.len()].copy_from_slice(version.as_bytes());
        }
        for offset in 0..disc::TOC_SECTORS as usize {
            put_user_sector(
                &mut combined,
                disc::TOC_LBA as usize + offset,
                &toc[offset * USER_DATA_BYTES..(offset + 1) * USER_DATA_BYTES],
            );
        }
        let combined_bin = root.join("combined.bin");
        let combined_cue = root.join("combined.cue");
        fs::write(&combined_bin, &combined).unwrap();
        fs::write(
            &combined_cue,
            "FILE \"combined.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n",
        )
        .unwrap();
        Fixture {
            _dir: dir,
            root,
            names: names.iter().map(|n| n.to_string()).collect(),
            source,
            cook_manifest,
            frontend,
            programs,
            combined_bin,
            combined_cue,
        }
    }

    fn document(&self, names: Option<&[&str]>) -> Result<Value> {
        let names: Vec<String> = names
            .map(|n| n.iter().map(|s| s.to_string()).collect())
            .unwrap_or_else(|| self.names.clone());
        let programs: Vec<(String, PathBuf)> = self
            .programs
            .iter()
            .filter(|(n, _)| names.contains(n))
            .cloned()
            .collect();
        let sources: Vec<(String, PathBuf)> = names
            .iter()
            .map(|n| (n.clone(), self.source.clone()))
            .collect();
        build_document(
            &self.combined_cue,
            &self.frontend,
            "make disc HL=1 CS=0 HK=0 DIST=dist/hardware-candidate",
            &programs,
            &sources,
        )
    }
}

fn fails_with(result: Result<Value>, needle: &str) {
    let error = result.expect_err("should fail closed").0;
    assert!(
        error.contains(needle),
        "{error:?} does not contain {needle:?}"
    );
}

#[test]
fn records_and_verifies_all_program_artifacts() {
    let fixture = Fixture::new(&HL_PRESSING);
    let document = fixture.document(None).unwrap();
    let programs = document["programs"].as_object().unwrap();
    assert_eq!(
        programs.keys().cloned().collect::<BTreeSet<_>>(),
        HL_PRESSING.iter().map(|s| s.to_string()).collect()
    );
    for row in programs.values() {
        assert_eq!(row["source"]["tree_clean"], json!(true));
        assert_eq!(row["embedded"]["image_sectors"], json!(4));
        assert_eq!(row["input"]["payload"]["bytes"], json!(2048));
    }
    let cooked = &programs["HALF-LIFE"]["cooked_assets"];
    assert_eq!(cooked["verified"], json!(true));
    assert_eq!(cooked["document"]["schema"], json!(HL_COOK_SCHEMA));
}

#[test]
fn counter_strike_and_hollow_knight_without_half_life() {
    let names = [
        "CORTEX IGNITION",
        "QUAKE SHAREWARE",
        "COUNTER-STRIKE",
        "HOLLOW KNIGHT",
    ];
    let fixture = Fixture::new(&names);
    let document = fixture.document(None).unwrap();
    let programs = document["programs"].as_object().unwrap();
    assert_eq!(programs.len(), 4);
    assert!(programs["COUNTER-STRIKE"].get("cooked_assets").is_none());
}

#[test]
fn a_pressed_optional_program_must_be_receipted() {
    let names = [
        "CORTEX IGNITION",
        "QUAKE SHAREWARE",
        "HALF-LIFE",
        "HOLLOW KNIGHT",
    ];
    let fixture = Fixture::new(&names);
    let error = fixture.document(Some(&HL_PRESSING)).unwrap_err().0;
    assert!(
        error.contains("does not cover") && error.contains("HOLLOW KNIGHT"),
        "{error}"
    );
}

#[test]
fn unknown_or_missing_programs_are_refused() {
    let extra = pressed_programs(
        &[
            "CORTEX IGNITION".into(),
            "QUAKE SHAREWARE".into(),
            "DOOM".into(),
        ],
        "program",
    );
    assert!(extra.unwrap_err().0.contains("extra=['DOOM']"));
    let missing = pressed_programs(
        &["CORTEX IGNITION".into(), "HARDWARE TESTS".into()],
        "program",
    );
    assert!(missing
        .unwrap_err()
        .0
        .contains("missing=['QUAKE SHAREWARE']"));
}

#[test]
fn half_life_cooked_asset_tamper_fails_closed() {
    let fixture = Fixture::new(&HL_PRESSING);
    fs::write(fixture.source.join("data/chunk.bin"), b"stale mutation").unwrap();
    fails_with(
        fixture.document(None),
        "does not match the current cooked assets",
    );
}

#[test]
fn half_life_manifest_revision_fails_closed() {
    let fixture = Fixture::new(&HL_PRESSING);
    let mut document: Value =
        serde_json::from_slice(&fs::read(&fixture.cook_manifest).unwrap()).unwrap();
    document["hl_psx_revision"] = json!("0".repeat(40));
    fs::write(&fixture.cook_manifest, document.to_string()).unwrap();
    fails_with(fixture.document(None), "revision does not match");
}

#[test]
fn dirty_source_fails_closed() {
    let fixture = Fixture::new(&HL_PRESSING);
    fs::write(fixture.source.join("untracked.txt"), "dirty\n").unwrap();
    fails_with(fixture.document(None), "source is dirty");
}

#[test]
fn embedded_tamper_fails_closed() {
    let fixture = Fixture::new(&HL_PRESSING);
    let mut image = fs::read(&fixture.combined_bin).unwrap();
    image[30 * SECTOR_BYTES as usize + 100] ^= 1;
    fs::write(&fixture.combined_bin, image).unwrap();
    fails_with(fixture.document(None), "embedded sector");
}

#[test]
fn msf_bytes_may_differ_but_nothing_else() {
    let fixture = Fixture::new(&HL_PRESSING);
    let mut image = fs::read(&fixture.combined_bin).unwrap();
    image[30 * SECTOR_BYTES as usize + 13] ^= 1;
    fs::write(&fixture.combined_bin, &image).unwrap();
    fixture.document(None).unwrap();
    image[30 * SECTOR_BYTES as usize + 15] ^= 1;
    fs::write(&fixture.combined_bin, &image).unwrap();
    fails_with(fixture.document(None), "embedded sector");
}

#[test]
fn audio_tracks_are_hashed_but_only_data_track_is_embedded() {
    let fixture = Fixture::new(&HL_PRESSING);
    let (_, cue) = &fixture.programs[0];
    let image = image_for_cue(cue).unwrap();
    let mut bytes = fs::read(&image).unwrap();
    bytes.extend(vec![0xA5u8; 2 * SECTOR_BYTES as usize]);
    fs::write(&image, bytes).unwrap();
    fs::write(
        cue,
        format!(
            "FILE \"{}\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    INDEX 00 00:00:04\n    INDEX 01 00:02:04\n",
            image.file_name().unwrap().to_string_lossy()
        ),
    )
    .unwrap();
    let document = fixture.document(None).unwrap();
    let row = &document["programs"][HL_PRESSING[0]];
    assert_eq!(row["input"]["bin_total_sectors"], json!(6));
    assert_eq!(row["input"]["data_track_sectors"], json!(4));
    assert_eq!(row["embedded"]["image_sectors"], json!(4));
}

#[test]
fn sealed_verification_survives_deleted_build_inputs() {
    let fixture = Fixture::new(&HL_PRESSING);
    let document = fixture.document(None).unwrap();
    let receipt_path = fixture.root.join("combined.release-receipt.json");
    fs::write(&receipt_path, document.to_string()).unwrap();
    fs::remove_file(&fixture.frontend).unwrap();
    for (_, cue) in &fixture.programs {
        fs::remove_file(image_for_cue(cue).unwrap()).unwrap();
        fs::remove_file(cue).unwrap();
    }
    verify_sealed_document(&receipt_path, &document).unwrap();
}

#[test]
fn sealed_verification_rejects_combined_tamper() {
    let fixture = Fixture::new(&HL_PRESSING);
    let document = fixture.document(None).unwrap();
    let receipt_path = fixture.root.join("combined.release-receipt.json");
    fs::write(&receipt_path, document.to_string()).unwrap();
    let mut image = fs::read(&fixture.combined_bin).unwrap();
    *image.last_mut().unwrap() ^= 1;
    fs::write(&fixture.combined_bin, image).unwrap();
    let error = verify_sealed_document(&receipt_path, &document)
        .unwrap_err()
        .0;
    assert!(error.contains("SHA-256"), "{error}");
}

#[test]
fn verify_rebuilds_the_document_and_notices_drift() {
    let fixture = Fixture::new(&HL_PRESSING);
    let document = fixture.document(None).unwrap();
    let receipt_path = fixture.root.join("combined.release-receipt.json");
    write_receipt(&receipt_path, &document).unwrap();
    let args = |extra: &[&str]| -> Args {
        let mut raw = vec!["--receipt".to_string(), receipt_path.display().to_string()];
        raw.extend(extra.iter().map(|s| s.to_string()));
        Args::parse(&raw, &["receipt"], &["sealed"]).unwrap()
    };
    verify(&args(&[])).unwrap();
    fs::write(&fixture.frontend, b"a different frontend").unwrap();
    assert!(verify(&args(&[]))
        .unwrap_err()
        .0
        .contains("no longer matches"));
}

#[test]
fn tree_hash_orders_paths_by_component() {
    // "a/b" sorts before "a.c" by path component but after it as a string; the
    // digest has to follow the component order, as it always did.
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("a")).unwrap();
    fs::write(dir.path().join("a/b"), b"1").unwrap();
    fs::write(dir.path().join("a.c"), b"2").unwrap();
    let one = tree_sha256(dir.path(), &[PathBuf::from("a/b"), PathBuf::from("a.c")]).unwrap();
    let two = tree_sha256(dir.path(), &[PathBuf::from("a.c"), PathBuf::from("a/b")]).unwrap();
    assert_eq!(one, two);
    let mut expected = Sha256::new();
    expected.update(TREE_DOMAIN);
    for (path, body) in [("a/b", b"1"), ("a.c", b"2")] {
        expected.update((path.len() as u64).to_le_bytes());
        expected.update(path.as_bytes());
        expected.update(1u64.to_le_bytes());
        expected.update(body);
    }
    assert_eq!(one, hex(&expected.finalize()));
}
