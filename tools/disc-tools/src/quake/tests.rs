//! Ports of the original Python suite's verifier, receipt and headless tests
//! (`VerifyQuakeTests` and `HeadlessChainloadTests`). The ones that drive
//! `make` live in `tests/quake_make.rs`, since they need the built binary.

use std::fs;
use std::path::Path;

use regex::Regex;
use serde_json::{json, Value};

use super::fixture::{digest, git, QuakeFixture, SECTOR_BYTES};
use super::verify::{verify_quake, Verified, VerifyInputs};
use super::write_receipt;
use crate::util::Result;

fn inputs(fixture: &QuakeFixture) -> VerifyInputs {
    VerifyInputs {
        source: fixture.source.clone(),
        psoxide: fixture.psoxide.clone(),
        programs_psoxide: None,
        programs_psoxide_stamp: fixture.programs_stamp.clone(),
        cue: fixture.cue.clone(),
        provenance: fixture.provenance.clone(),
        expected_revision: fixture.revision.clone(),
        expected_psoxide_revision: fixture.psoxide_revision.clone(),
        expected_programs_psoxide_revision: None,
        expected_provenance_sha256: fixture.provenance_sha256.clone(),
        expected_cue_sha256: digest(&fixture.cue),
        expected_bin_sha256: digest(&fixture.bin),
        expected_exe_sha256: digest(&fixture.exe),
    }
}

fn verify(fixture: &QuakeFixture) -> Result<Verified> {
    verify_quake(&inputs(fixture))
}

/// The failure message, or a test failure if the call succeeded.
fn failure<T>(result: Result<T>) -> String {
    match result {
        Ok(_) => panic!("expected a failure, but the call succeeded"),
        Err(error) => error.to_string(),
    }
}

fn assert_fails<T>(result: Result<T>, pattern: &str) {
    let message = failure(result);
    assert!(
        Regex::new(pattern).unwrap().is_match(&message),
        "message {message:?} does not match {pattern:?}"
    );
}

#[test]
fn accepts_exact_clean_revision_and_image() {
    let fixture = QuakeFixture::new();
    let verified = verify(&fixture).unwrap();
    assert_eq!(verified.source_revision, fixture.revision);
    assert_eq!(verified.psoxide_revision, fixture.psoxide_revision);
    assert_eq!(verified.declared_psoxide_revision, fixture.psoxide_revision);
    assert_eq!(verified.bin_bytes, 24 * SECTOR_BYTES as u64);
}

#[test]
fn accepts_separate_clean_sdk_for_ordinary_programs() {
    let fixture = QuakeFixture::new();
    let programs = fixture.root.join("programs-psoxide");
    fs::create_dir(&programs).unwrap();
    git(&programs, &["init", "-q"]);
    git(&programs, &["config", "user.name", "Test"]);
    git(&programs, &["config", "user.email", "test@example.invalid"]);
    git(&programs, &["config", "commit.gpgsign", "false"]);
    fs::write(programs.join("sdk.txt"), "shared runtime\n").unwrap();
    git(&programs, &["add", "sdk.txt"]);
    git(&programs, &["commit", "-q", "-m", "fixture"]);
    let programs_revision = git(&programs, &["rev-parse", "HEAD"]);
    fs::write(&fixture.programs_stamp, format!("{programs_revision}\n")).unwrap();

    let mut wanted = inputs(&fixture);
    wanted.programs_psoxide = Some(programs);
    wanted.expected_programs_psoxide_revision = Some(programs_revision.clone());
    let verified = verify_quake(&wanted).unwrap();
    assert_eq!(verified.psoxide_revision, fixture.psoxide_revision);
    assert_eq!(verified.programs_psoxide_revision, programs_revision);
}

#[test]
fn rejects_component_provenance_drift() {
    let mut fixture = QuakeFixture::new();
    let mut document: Value =
        serde_json::from_str(&fs::read_to_string(&fixture.provenance).unwrap()).unwrap();
    document["psoxide"]["components"]["sdk"]["revision"] = json!("c".repeat(40));
    fixture.write_and_pin_provenance(&document);
    assert_fails(verify(&fixture), "component provenance differs");
}

#[test]
fn rejects_wrong_revision() {
    let fixture = QuakeFixture::new();
    let mut wanted = inputs(&fixture);
    wanted.expected_revision = "0".repeat(40);
    assert_fails(verify_quake(&wanted), "revision mismatch");
}

#[test]
fn rejects_dirty_source() {
    let fixture = QuakeFixture::new();
    fs::write(fixture.source.join("tracked.txt"), "changed\n").unwrap();
    assert_fails(verify(&fixture), "source checkout is dirty");
}

#[test]
fn rejects_quake_and_psoxide_revision_mismatch() {
    let mut fixture = QuakeFixture::new();
    fixture.advance_psoxide();
    assert_fails(verify(&fixture), "PSOXIDE_REV mismatch");
}

#[test]
fn rejects_dirty_psoxide_checkout() {
    let fixture = QuakeFixture::new();
    fs::write(fixture.psoxide.join("sdk.txt"), "dirty\n").unwrap();
    assert_fails(verify(&fixture), "PSoXide checkout is dirty");
}

#[test]
fn rejects_malformed_quake_psoxide_revision() {
    let mut fixture = QuakeFixture::new();
    fixture.commit_quake_declaration("const PSOXIDE_REV: &str = \"not-a-full-revision\";\n");
    assert_fails(verify(&fixture), "Quake PSOXIDE_REV");
}

#[test]
fn rejects_uppercase_quake_psoxide_revision() {
    let mut fixture = QuakeFixture::new();
    let declaration = format!(
        "const PSOXIDE_REV: &str = \"{}\";\n",
        fixture.psoxide_revision.to_uppercase()
    );
    fixture.commit_quake_declaration(&declaration);
    assert_fails(verify(&fixture), "full lowercase hexadecimal");
}

#[test]
fn rejects_mismatched_ordinary_program_revision_stamp() {
    let fixture = QuakeFixture::new();
    fs::write(&fixture.programs_stamp, format!("{}\n", "0".repeat(40))).unwrap();
    assert_fails(verify(&fixture), "ordinary-program SDK revision mismatch");
}

#[test]
fn rejects_changed_bin() {
    let fixture = QuakeFixture::new();
    let expected_bin_hash = digest(&fixture.bin);
    // Flip the last byte, keeping the size, so only the hash can notice.
    let mut bytes = fs::read(&fixture.bin).unwrap();
    *bytes.last_mut().unwrap() = b'x';
    fs::write(&fixture.bin, bytes).unwrap();
    let mut wanted = inputs(&fixture);
    wanted.expected_bin_sha256 = expected_bin_hash;
    assert_fails(verify_quake(&wanted), "bin SHA-256 mismatch");
}

#[test]
fn rejects_cue_that_escapes_its_directory() {
    let fixture = QuakeFixture::new();
    fs::write(
        &fixture.cue,
        "FILE \"../quake-psx.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n",
    )
    .unwrap();
    assert_fails(verify(&fixture), "beside the cue");
}

#[test]
fn rejects_missing_malformed_and_duplicate_key_sidecars() {
    let cases = [
        ("missing", None, "sidecar does not exist"),
        ("malformed", Some("{"), "cannot parse"),
        (
            "duplicate",
            Some("{\"schema\": 1, \"schema\": 1}"),
            "duplicate key",
        ),
    ];
    for (label, contents, error) in cases {
        let fixture = QuakeFixture::new();
        match contents {
            None => fs::remove_file(&fixture.provenance).unwrap(),
            Some(text) => fs::write(&fixture.provenance, text).unwrap(),
        }
        let message = failure(verify(&fixture));
        assert!(
            Regex::new(error).unwrap().is_match(&message),
            "{label}: {message}"
        );
    }
}

#[test]
fn rejects_sidecar_not_beside_cue_with_exact_name() {
    let fixture = QuakeFixture::new();
    let alternate = fixture.cue.parent().unwrap().join("shipping.json");
    fs::rename(&fixture.provenance, &alternate).unwrap();
    let mut wanted = inputs(&fixture);
    wanted.provenance = alternate;
    assert_fails(verify_quake(&wanted), "must be beside the cue and named");

    let fixture = QuakeFixture::new();
    let elsewhere = fixture.root.join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    let alternate = elsewhere.join(fixture.provenance.file_name().unwrap());
    fs::rename(&fixture.provenance, &alternate).unwrap();
    let mut wanted = inputs(&fixture);
    wanted.provenance = alternate;
    assert_fails(verify_quake(&wanted), "must be beside the cue and named");
}

/// Mutate the fixture's sidecar and expect the verifier to refuse it.
fn mutated_sidecar_fails(label: &str, error: &str, mutate: impl FnOnce(&QuakeFixture, &mut Value)) {
    let fixture = QuakeFixture::new();
    let mut document = fixture.provenance_document();
    mutate(&fixture, &mut document);
    fixture.write_provenance(&document);
    let message = failure(verify(&fixture));
    assert!(
        Regex::new(error).unwrap().is_match(&message),
        "{label}: {message:?} vs {error:?}"
    );
}

#[test]
fn rejects_stale_sidecar_revisions_and_source_kind() {
    mutated_sidecar_fails(
        "quake revision",
        "provenance Quake revision mismatch",
        |_, d| {
            d["quake_source"]["revision"] = json!("0".repeat(40));
        },
    );
    mutated_sidecar_fails(
        "psoxide revision",
        "provenance PSoXide revision mismatch",
        |_, d| {
            d["psoxide"]["revision"] = json!("0".repeat(40));
        },
    );
    mutated_sidecar_fails("source kind", "source kind", |_, d| {
        d["psoxide"]["source_kind"] = json!("remote");
    });
    mutated_sidecar_fails("dirty claim", "clean Quake source tree", |_, d| {
        d["quake_source"]["tree_clean"] = json!(false);
    });
}

#[test]
fn accepts_pinned_psoxide_hydration() {
    let mut fixture = QuakeFixture::new();
    let mut document = fixture.provenance_document();
    document["psoxide"]["source_kind"] = json!("pinned_hydration");
    fixture.write_and_pin_provenance(&document);
    verify(&fixture).unwrap();
}

#[test]
fn rejects_wrong_shareware_and_nonshipping_build_contracts() {
    mutated_sidecar_fails("pak hash", "canonical Quake 1.06 shareware", |_, d| {
        d["shareware"]["pak0_sha256"] = json!("0".repeat(64));
    });
    mutated_sidecar_fails("pak size", "canonical Quake 1.06 shareware", |_, d| {
        d["shareware"]["pak0_bytes"] = json!(1);
    });
    mutated_sidecar_fails("recipe missing", "missing required field", |_, d| {
        d["build"]
            .as_object_mut()
            .unwrap()
            .remove("guest_recipe_sha256");
    });
    mutated_sidecar_fails("toolchain malformed", "Rust toolchain SHA-256", |_, d| {
        d["build"]["rust_toolchain_sha256"] = json!("not-a-hash");
    });
    mutated_sidecar_fails("profile", "release profile", |_, d| {
        d["build"]["profile"] = json!("dev");
    });
    mutated_sidecar_fails("features", "release profile", |_, d| {
        d["build"]["features"] = json!(["emulator-telemetry"]);
    });
}

#[test]
fn rejects_artifact_name_size_hash_and_byte_drift() {
    mutated_sidecar_fails("exe basename", "names .* expected", |f, d| {
        d["artifacts"]["exe"]["file"] = json!(f.bin.file_name().unwrap().to_string_lossy());
    });
    mutated_sidecar_fails("cue size", "byte-size mismatch", |f, d| {
        d["artifacts"]["cue"]["bytes"] = json!(fs::metadata(&f.cue).unwrap().len() + 1);
    });
    mutated_sidecar_fails(
        "bin sidecar hash",
        "artifacts.bin SHA-256 mismatch",
        |_, d| {
            d["artifacts"]["bin"]["sha256"] = json!("0".repeat(64));
        },
    );
    mutated_sidecar_fails("exe bytes", "artifacts.exe.*mismatch", |f, _| {
        fs::write(&f.exe, b"changed").unwrap();
    });
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn receipt_hashes_input_and_output_and_keeps_legal_gate() {
    let fixture = QuakeFixture::new();
    let verified = verify(&fixture).unwrap();
    let (demo_cue, demo_bin) = fixture.make_demo();
    let receipt_path = fixture.root.join("receipt.json");
    write_receipt(&demo_cue, &demo_bin, &receipt_path, &verified).unwrap();
    let receipt = read_json(&receipt_path);
    assert_eq!(
        receipt["quake_input"]["source_revision"],
        json!(fixture.revision)
    );
    assert_eq!(receipt["quake_input"]["source_tree_clean"], json!(true));
    assert_eq!(
        receipt["quake_input"]["declared_psoxide_revision"],
        json!(fixture.psoxide_revision)
    );
    assert_eq!(
        receipt["psoxide_input"]["revision"],
        json!(fixture.psoxide_revision)
    );
    assert_eq!(receipt["psoxide_input"]["tree_clean"], json!(true));
    assert_eq!(
        receipt["psoxide_input"]["matches_quake_declared_revision"],
        json!(true)
    );
    assert_eq!(
        receipt["psoxide_input"]["ordinary_programs_revision"],
        json!(fixture.psoxide_revision)
    );
    assert_eq!(
        receipt["psoxide_input"]["ordinary_programs_match_checkout"],
        json!(true)
    );
    assert_eq!(
        receipt["psoxide_input"]["ordinary_programs_match_quake_sdk"],
        json!(true)
    );
    assert_eq!(
        receipt["quake_artifact_sdk_provenance"]["status"],
        json!("sidecar-bound")
    );
    assert_eq!(receipt["schema"], json!(3));
    assert_eq!(
        receipt["quake_artifact_sdk_provenance"]["build"]["guest_recipe_sha256"],
        json!("1".repeat(64))
    );
    assert_eq!(
        receipt["quake_input"]["bin_sha256"],
        json!(digest(&fixture.bin))
    );
    assert_eq!(
        receipt["quake_input"]["exe_sha256"],
        json!(digest(&fixture.exe))
    );
    assert_eq!(
        receipt["quake_input"]["provenance_sha256"],
        json!(digest(&fixture.provenance))
    );
    assert_eq!(
        receipt["demo_disc_output"]["bin_sha256"],
        json!(digest(&demo_bin))
    );
    assert_eq!(
        receipt["demo_disc_output"]["embedded_quake_matches_input_except_msf"],
        json!(true)
    );
    assert_eq!(
        receipt["demo_disc_output"]["quake_toc"]["image_lba_offset"],
        json!(30)
    );
    assert_eq!(
        receipt["redistribution"],
        json!(
            "owner-approved public non-commercial release of canonical \
             Quake 1.06 shareware payload, 2026-08-25"
        )
    );
}

/// Overwrite one byte of the demo bin, then expect the receipt to refuse it.
fn receipt_refuses_byte(offset: usize, value: u8) {
    let fixture = QuakeFixture::new();
    let verified = verify(&fixture).unwrap();
    let (demo_cue, demo_bin) = fixture.make_demo();
    let mut bytes = fs::read(&demo_bin).unwrap();
    bytes[30 * SECTOR_BYTES + offset] = value;
    fs::write(&demo_bin, bytes).unwrap();
    let out = fixture.root.join("x.json");
    assert_fails(
        write_receipt(&demo_cue, &demo_bin, &out, &verified),
        "differs outside",
    );
    assert!(!out.exists(), "a refused receipt must not be written");
}

#[test]
fn receipt_rejects_embedded_image_drift() {
    receipt_refuses_byte(100, b'x');
}

#[test]
fn receipt_rejects_mode_byte_drift() {
    receipt_refuses_byte(15, 1);
}
