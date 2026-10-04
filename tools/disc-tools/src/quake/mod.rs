//! `disc-tools quake`: verify and record the Quake shareware payload every
//! pressing carries, print the pins a built Quake tree implies, and replay
//! the chain-load headless.
//!
//! * `verify` proves the pinned Quake and PSoXide checkouts, the cue, bin and
//!   exe and the provenance sidecar all agree with the Makefile's pins.
//! * `receipt` verifies again, then proves the pressed disc carries that bin
//!   and writes the receipt that says so.
//! * `repin` only prints; the pins are edited by hand so a diff shows which
//!   contract moved.
//! * `headless` is the older two-replay chain-load check.

mod headless;
mod json;
mod verify;

#[cfg(test)]
mod fixture;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::args::Args;
use crate::util::{self, git, object, resolve, sha256_file, Result};
use verify::{
    cue_bin, declared_psoxide_revision, file_name, quake_toc_entry, verify_embedded_image,
    verify_quake, Verified, VerifyInputs, QUAKE_PROVENANCE_SCHEMA, REDISTRIBUTION_STATUS,
    SECTOR_BYTES,
};

const USAGE: &str = "usage: disc-tools quake <verify|receipt|repin|headless> [options]";

/// The options `verify` and `receipt` both read.
const VERIFY_OPTIONS: [&str; 13] = [
    "source",
    "psoxide",
    "programs-psoxide",
    "programs-psoxide-stamp",
    "cue",
    "provenance",
    "expected-revision",
    "expected-psoxide-revision",
    "expected-programs-psoxide-revision",
    "expected-provenance-sha256",
    "expected-cue-sha256",
    "expected-bin-sha256",
    "expected-exe-sha256",
];

pub fn run(args: &[String]) -> Result<i32> {
    let Some((command, rest)) = args.split_first() else {
        bail!("{USAGE}");
    };
    match command.as_str() {
        "verify" => verify_command(rest, false),
        "receipt" => verify_command(rest, true),
        "repin" => repin_command(rest),
        "headless" => headless::run(rest),
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            Ok(0)
        }
        other => bail!("unknown subcommand {other:?}; {USAGE}"),
    }
}

/// An empty option counts as absent, as the Makefile passes empty strings for
/// the programs checkout when it is the same as the disc's.
fn optional(args: &Args, name: &str) -> Option<String> {
    args.get(name)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn inputs_from(args: &Args) -> Result<VerifyInputs> {
    Ok(VerifyInputs {
        source: args.require_path("source")?,
        psoxide: args.require_path("psoxide")?,
        programs_psoxide: optional(args, "programs-psoxide").map(PathBuf::from),
        programs_psoxide_stamp: args.require_path("programs-psoxide-stamp")?,
        cue: args.require_path("cue")?,
        provenance: args.require_path("provenance")?,
        expected_revision: args.require("expected-revision")?.to_string(),
        expected_psoxide_revision: args.require("expected-psoxide-revision")?.to_string(),
        expected_programs_psoxide_revision: optional(args, "expected-programs-psoxide-revision"),
        expected_provenance_sha256: args.require("expected-provenance-sha256")?.to_string(),
        expected_cue_sha256: args.require("expected-cue-sha256")?.to_string(),
        expected_bin_sha256: args.require("expected-bin-sha256")?.to_string(),
        expected_exe_sha256: args.require("expected-exe-sha256")?.to_string(),
    })
}

fn verify_command(raw: &[String], receipt: bool) -> Result<i32> {
    let mut options = VERIFY_OPTIONS.to_vec();
    if receipt {
        options.extend(["demo-cue", "demo-bin", "out"]);
    }
    let args = Args::parse(raw, &options, &[])?;
    ensure!(
        args.positional.is_empty(),
        "unrecognized arguments: {}",
        args.positional.join(" ")
    );
    let inputs = inputs_from(&args)?;
    let verified = verify_quake(&inputs)?;
    print_verification(&verified);
    if receipt {
        let out = write_receipt(
            &args.require_path("demo-cue")?,
            &args.require_path("demo-bin")?,
            &args.require_path("out")?,
            &verified,
        )?;
        println!("quake provenance receipt: {}", out.display());
    }
    Ok(0)
}

fn print_verification(verified: &Verified) {
    println!("quake source revision: {}", verified.source_revision);
    println!(
        "quake declared PSoXide revision: {}",
        verified.declared_psoxide_revision
    );
    println!("disc PSoXide revision: {}", verified.psoxide_revision);
    println!(
        "ordinary-program PSoXide revision: {}",
        verified.programs_psoxide_revision
    );
    println!("quake provenance SHA-256: {}", verified.provenance_sha256);
    println!(
        "quake guest recipe SHA-256: {}",
        verified.guest_recipe_sha256
    );
    println!("quake cue SHA-256: {}", verified.cue_sha256);
    println!("quake bin SHA-256: {}", verified.bin_sha256);
    println!("quake bin bytes: {}", verified.bin_bytes);
    println!("quake exe SHA-256: {}", verified.exe_sha256);
    println!("quake exe bytes: {}", verified.exe_bytes);
}

fn text(value: &str) -> Value {
    Value::String(value.to_string())
}

/// Prove the pressed disc carries the verified bin, then write the receipt.
/// The three demo-side checks (the cue names the bin, the bin is whole
/// sectors, the table row is the pinned Quake) come before any hashing.
fn write_receipt(
    demo_cue: &Path,
    demo_bin: &Path,
    out: &Path,
    verified: &Verified,
) -> Result<PathBuf> {
    let demo_cue = resolve(demo_cue)?;
    let demo_bin = resolve(demo_bin)?;
    let referenced = cue_bin(&demo_cue, false)?;
    ensure!(
        referenced == demo_bin,
        "demo cue references {}, not requested output {}",
        referenced.display(),
        demo_bin.display()
    );
    let demo_size = std::fs::metadata(&demo_bin)?.len();
    ensure!(
        demo_size != 0 && demo_size % SECTOR_BYTES == 0,
        "demo bin is {demo_size} bytes, not a non-empty whole number of {SECTOR_BYTES}-byte sectors"
    );
    let toc_entry = quake_toc_entry(&demo_bin, &verified.source_revision)?;
    let embedded_sectors = verify_embedded_image(&demo_bin, &verified.bin, toc_entry.lba_offset)?;

    let quake_input = object([
        ("source_revision", text(&verified.source_revision)),
        ("source_tree_clean", Value::Bool(true)),
        (
            "declared_psoxide_revision",
            text(&verified.declared_psoxide_revision),
        ),
        ("provenance_file", text(&file_name(&verified.provenance))),
        ("provenance_sha256", text(&verified.provenance_sha256)),
        ("cue_file", text(&file_name(&verified.cue))),
        ("cue_sha256", text(&verified.cue_sha256)),
        ("cue_bytes", Value::from(verified.cue_bytes)),
        ("bin_file", text(&file_name(&verified.bin))),
        ("bin_sha256", text(&verified.bin_sha256)),
        ("bin_bytes", Value::from(verified.bin_bytes)),
        ("exe_file", text(&file_name(&verified.exe))),
        ("exe_sha256", text(&verified.exe_sha256)),
        ("exe_bytes", Value::from(verified.exe_bytes)),
    ]);
    let psoxide_input = object([
        ("revision", text(&verified.psoxide_revision)),
        ("tree_clean", Value::Bool(true)),
        ("matches_quake_declared_revision", Value::Bool(true)),
        (
            "ordinary_programs_revision",
            text(&verified.programs_psoxide_revision),
        ),
        ("ordinary_programs_match_checkout", Value::Bool(true)),
        (
            "ordinary_programs_match_quake_sdk",
            Value::Bool(verified.programs_psoxide_revision == verified.psoxide_revision),
        ),
    ]);
    let build = object([
        (
            "guest_stage_schema",
            Value::from(verified.guest_stage_schema as i64),
        ),
        ("guest_recipe_sha256", text(&verified.guest_recipe_sha256)),
        (
            "rust_toolchain_sha256",
            text(&verified.rust_toolchain_sha256),
        ),
        ("rustc_version", text(&verified.rustc_version)),
        ("cargo_version", text(&verified.cargo_version)),
        ("profile", text(&verified.profile)),
        (
            "features",
            Value::Array(verified.features.iter().map(|f| text(f)).collect()),
        ),
    ]);
    let sdk_provenance = object([
        ("status", text("sidecar-bound")),
        ("schema", Value::from(QUAKE_PROVENANCE_SCHEMA as i64)),
        ("psoxide_source_kind", text(&verified.psoxide_source_kind)),
        (
            "shareware",
            object([
                ("pak0_sha256", text(&verified.pak0_sha256)),
                ("pak0_bytes", Value::from(verified.pak0_bytes as i64)),
            ]),
        ),
        ("build", build),
        (
            "proved",
            text(
                "the clean Quake and PSoXide revisions, canonical shareware PAK, guest build recipe, \
                 toolchain identities, and actual cue/bin/exe bytes match the shipping sidecar",
            ),
        ),
    ]);
    let quake_toc = object([
        ("exe_lba", Value::from(toc_entry.exe_lba)),
        ("image_lba_offset", Value::from(toc_entry.lba_offset)),
        ("cdda_track_base", Value::from(toc_entry.cdda_track_base)),
        (
            "payload_fnv1a32",
            Value::String(format!("0x{:08x}", toc_entry.payload_fnv)),
        ),
        ("menu_version", text(&toc_entry.version)),
    ]);
    let demo_output = object([
        ("cue_file", text(&file_name(&demo_cue))),
        ("cue_sha256", text(&sha256_file(&demo_cue)?)),
        ("bin_file", text(&file_name(&demo_bin))),
        ("bin_sha256", text(&sha256_file(&demo_bin)?)),
        ("bin_bytes", Value::from(demo_size)),
        ("quake_toc", quake_toc),
        ("embedded_quake_data_sectors", Value::from(embedded_sectors)),
        ("embedded_quake_matches_input_except_msf", Value::Bool(true)),
    ]);
    let receipt = object([
        ("schema", Value::from(3)),
        ("variant", text("quake-shareware-default")),
        ("redistribution", text(REDISTRIBUTION_STATUS)),
        ("quake_input", quake_input),
        ("psoxide_input", psoxide_input),
        ("quake_artifact_sdk_provenance", sdk_provenance),
        ("demo_disc_output", demo_output),
    ]);
    util::write_receipt(out, &receipt)?;
    Ok(out.to_path_buf())
}

/// Print what a built Quake tree implies, as the lines to paste into the
/// Makefile. Deliberately writes nothing: the pins are edited by hand so the
/// diff shows which contract moved, and a repin that silently rewrote them
/// would be a verifier that agrees with whatever it is given.
fn repin_command(raw: &[String]) -> Result<i32> {
    let args = Args::parse(raw, &["source", "cue", "provenance"], &[])?;
    ensure!(
        args.positional.is_empty(),
        "unrecognized arguments: {}",
        args.positional.join(" ")
    );
    let source = args.require_path("source")?;
    let source = match source.canonicalize() {
        Ok(real) => real,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            bail!("Quake source does not exist: {}", source.display())
        }
        Err(e) => bail!("{}: {e}", source.display()),
    };
    let revision = git(&source, &["rev-parse", "--verify", "HEAD^{commit}"])?.to_lowercase();
    let dirty = git(
        &source,
        &["status", "--porcelain=v1", "--untracked-files=normal"],
    )?;
    let declared = declared_psoxide_revision(&source)?;
    let cue = resolve(&args.require_path("cue")?)?;
    let bin = cue_bin(&cue, true)?;
    let exe = cue.with_extension("exe");
    let provenance = resolve(&args.require_path("provenance")?)?;

    println!("# measured from {}", source.display());
    println!("QUAKE_EXPECTED_REV ?= {revision}");
    println!("QUAKE_EXPECTED_PSOXIDE_REV ?= {declared}");
    println!(
        "QUAKE_EXPECTED_PROVENANCE_SHA256 ?= {}",
        sha256_file(&provenance)?
    );
    println!("QUAKE_EXPECTED_CUE_SHA256 ?= {}", sha256_file(&cue)?);
    println!("QUAKE_EXPECTED_BIN_SHA256 ?= {}", sha256_file(&bin)?);
    println!("QUAKE_EXPECTED_EXE_SHA256 ?= {}", sha256_file(&exe)?);
    println!();
    println!("# then, in this order:");
    println!("#   git -C games/PSoXide-editor checkout {declared} && git add games/PSoXide-editor");
    println!("#   make disc");
    println!("#   make quake-headless-check   (two deterministic replays, Quake selected by name)");
    if !dirty.is_empty() {
        println!();
        println!("# WARNING: the Quake tree is dirty, so these values name no revision.");
        println!("# Commit or clean it and measure again; quake-verify will reject them.");
    }
    Ok(0)
}
