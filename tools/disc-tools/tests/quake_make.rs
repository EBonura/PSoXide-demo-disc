//! Ports of the original `MakeVariantContractTests` and
//! `FailClosedDefaultTests`: the Makefile has to call the Quake verifier on
//! every path that lays out or checks a disc, and `make quake-verify` has to
//! fail closed on every way the payload can be wrong.
//!
//! The repo root has a space in its path on some checkouts and make cannot
//! carry one through an unquoted recipe, so every make runs with `ROOT` set to
//! a space-free symlink to the repo, and with `DISC_TOOLS` set to a symlink
//! to the binary this test run was built from (the Makefile would otherwise
//! try to rebuild a release binary first).

#[path = "../src/quake/fixture.rs"]
mod fixture;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use fixture::{digest, git, QuakeFixture};
use regex::Regex;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// A space-free way to name the repo and the binary under test.
struct Links {
    _guard: tempfile::TempDir,
    root: PathBuf,
    tool: PathBuf,
}

impl Links {
    fn new() -> Links {
        let guard = tempfile::Builder::new()
            .prefix("qk")
            .tempdir_in("/tmp")
            .expect("temp dir");
        let base = guard.path().canonicalize().unwrap();
        let (root, tool) = (base.join("root"), base.join("disc-tools"));
        symlink(repo_root(), &root).unwrap();
        symlink(env!("CARGO_BIN_EXE_disc-tools"), &tool).unwrap();
        Links {
            _guard: guard,
            root,
            tool,
        }
    }

    fn make(&self, args: &[&str], assignments: &[(&str, &str)]) -> Output {
        let mut command = Command::new("make");
        command
            .current_dir(repo_root())
            .arg("--no-print-directory")
            .args(args);
        command.arg(format!("ROOT={}", self.root.display()));
        command.arg(format!("DISC_TOOLS={}", self.tool.display()));
        for (key, value) in assignments {
            command.arg(format!("{key}={value}"));
        }
        command.output().expect("make starts")
    }

    /// `make -n disc-only DIST=...` with extra assignments, stdout on success.
    fn dry_run(&self, target: &str, assignments: &[(&str, &str)]) -> String {
        let mut all = vec![("DIST", "/tmp/psoxide-quake-contract")];
        all.extend_from_slice(assignments);
        let output = self.make(&["-n", target], &all);
        assert!(
            output.status.success(),
            "make -n {target}: {}",
            text(&output.stderr)
        );
        text(&output.stdout)
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path.as_ref()).unwrap_or_else(|e| panic!("{}: {e}", path.as_ref().display()))
}

fn makefile() -> String {
    read(repo_root().join("Makefile"))
}

/// The menu version the disc derives from the Quake pin. Read from the
/// Makefile rather than hard coded: a repin is a routine event, and a literal
/// here turns every repin into a spurious test failure instead of a real one.
fn pinned_menu_version() -> String {
    for line in makefile().lines() {
        if line.starts_with("QUAKE_EXPECTED_REV") {
            let revision = line.split_once('=').unwrap().1.trim();
            return format!("q{}", &revision[..7]);
        }
    }
    panic!("Makefile has no QUAKE_EXPECTED_REV");
}

fn index_of(haystack: &str, needle: &str) -> usize {
    haystack
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not found"))
}

#[test]
fn default_disc_carries_quake_with_metadata_verifier_and_receipt() {
    let links = Links::new();
    let default = links.dry_run("disc-only", &[]);
    assert!(default.contains("--image \"QUAKE SHAREWARE="));
    assert!(
        Regex::new(r#"--shot "QUAKE SHAREWARE=[^"\n]*/quake-menu\.shot""#)
            .unwrap()
            .is_match(&default)
    );
    assert!(
        Regex::new(r#"--shot "QUAKE SHAREWARE=[^"\n]*/quake-gameplay\.shot""#)
            .unwrap()
            .is_match(&default)
    );
    assert!(default.contains(&format!(
        "--version-of \"QUAKE SHAREWARE={}\"",
        pinned_menu_version()
    )));
    assert!(default.contains("--describe \"QUAKE SHAREWARE="));
    // Was `tools/quake_disc.py verify` and `... receipt`.
    assert!(default.contains("disc-tools quake verify"));
    assert!(default.contains("disc-tools quake receipt"));
    for argument in [
        "--psoxide \"",
        "--programs-psoxide \"",
        "--programs-psoxide-stamp \"",
        "--expected-psoxide-revision \"",
        "--expected-programs-psoxide-revision \"",
        "--provenance \"",
        "--expected-provenance-sha256 \"",
        "--expected-exe-sha256 \"",
    ] {
        assert!(default.contains(argument), "{argument}");
    }
    assert!(default.contains("PSoXide Demo Disc.bin"));
    assert!(!default.contains("PSoXide Demo Disc Quake Shareware.bin"));
}

#[test]
fn the_opt_in_switch_is_gone() {
    let links = Links::new();
    let default = links.dry_run("disc-only", &[]);
    for assignment in ["QUAKE=", "QUAKE=0", "QUAKE=1"] {
        let (key, value) = assignment.split_once('=').unwrap();
        assert_eq!(
            default,
            links.dry_run("disc-only", &[(key, value)]),
            "{assignment}"
        );
    }
}

/// The games `make programs` builds through a sub-make of their own.
const SUB_MAKE_GAMES: [&str; 5] = [
    "voxide",
    "nitroxide",
    "psxcel",
    "pico8-psx",
    "psoxide-arcade",
];

/// A games directory for a dry run of `make disc`. A checkout without those
/// submodules cannot recurse into them, so the missing ones are replaced by
/// stubs whose Makefile accepts any target; everything else is the real thing.
/// Returns the directory to pass as `GAMES`, or `None` when the checkout has
/// every submodule and needs no help.
fn games_for_dry_run(links: &Links) -> Option<PathBuf> {
    let games = repo_root().join("games");
    if SUB_MAKE_GAMES
        .iter()
        .all(|name| games.join(name).join("Makefile").is_file())
    {
        return None;
    }
    let stubs = links.root.parent().unwrap().join("games");
    fs::create_dir(&stubs).unwrap();
    for entry in fs::read_dir(&games).unwrap() {
        let name = entry.unwrap().file_name();
        if !SUB_MAKE_GAMES.contains(&name.to_string_lossy().as_ref()) {
            symlink(games.join(&name), stubs.join(&name)).unwrap();
        }
    }
    for name in SUB_MAKE_GAMES {
        fs::create_dir(stubs.join(name)).unwrap();
        fs::write(stubs.join(name).join("Makefile"), "%:\n\t@true\n").unwrap();
    }
    Some(stubs)
}

#[test]
fn full_disc_build_checks_sdk_coherence_after_programs() {
    let links = Links::new();
    let stubs = games_for_dry_run(&links);
    let games = stubs
        .as_ref()
        .map_or_else(|| links.root.join("games"), Clone::clone);
    let assignments: Vec<(&str, String)> = stubs
        .iter()
        .map(|path| ("GAMES", path.display().to_string()))
        .collect();
    let assignments: Vec<(&str, &str)> =
        assignments.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let quake = links.dry_run("disc", &assignments);
    let programs_at = index_of(&quake, "PSOXIDE_FROM=");
    // Was `tools/components.py --check --games`.
    let coherence_at = index_of(&quake, "disc-tools components --check --games");
    let stamp_at = index_of(&quake, "programs.psoxide-revision.tmp");
    let verify_at = index_of(&quake, "disc-tools quake verify");
    assert!(quake.contains(&format!("DIST={}/voxide/dist", games.display())));
    assert!(quake.contains(&format!("DIST={}/psoxide-arcade/dist", games.display())));
    assert!(programs_at < coherence_at);
    assert!(coherence_at < stamp_at);
    assert!(stamp_at < verify_at);
}

#[test]
fn lock_audit_includes_both_psoxide_inputs() {
    let script = read(repo_root().join("tools/check-locks.sh"));
    assert!(!script.contains("games/PSoXide "));
    assert!(script.contains("games/PSoXide-editor "));
}

#[test]
fn disc_runtime_crates_do_not_use_quakes_frozen_sdk() {
    let root = repo_root();
    for path in [
        "loader/Cargo.toml",
        "launcher/Cargo.toml",
        "tools/mkdisc/Cargo.toml",
    ] {
        let manifest = read(root.join(path));
        assert!(!manifest.contains("games/PSoXide/"), "{path}");
        let expected = if path.contains("mkdisc") {
            "games/PSoXide-sdk/"
        } else {
            "games/PSoXide-editor/sdk/"
        };
        assert!(manifest.contains(expected), "{path}");
    }
    // Collection geometry/catalog now have one engine owner. Neither
    // collection keeps a second production implementation.
    let manifest = read(root.join("launcher/Cargo.toml"));
    assert!(manifest.contains("package = \"psx-carousel\""));
    assert!(manifest.contains("package = \"psx-disc-toc\""));
    assert!(!manifest.contains("path = \"../carousel\""));
    assert!(!manifest.contains("path = \"../disc-toc\""));
    let launcher = read(root.join("launcher/src/main.rs"));
    assert!(!launcher.contains("games/PSoXide/assets/"));
    assert!(launcher.contains("games/PSoXide-editor/assets/"));
}

#[test]
fn quake_program_stamp_rejects_missing_malformed_and_stale() {
    let links = Links::new();
    let mut fixture = QuakeFixture::new();
    let build = fixture.root.join("build");
    let stamp = build.join("programs.psoxide-revision");
    let verify_stamp = |fixture: &QuakeFixture| {
        let psoxide = fixture.psoxide.display().to_string();
        links.make(
            &["quake-programs-verify"],
            &[
                ("PSOXIDE", &psoxide),
                ("PROGRAMS_PSOXIDE", &psoxide),
                ("BUILD", &build.display().to_string()),
            ],
        )
    };

    let missing = verify_stamp(&fixture);
    assert!(!missing.status.success());
    assert!(text(&missing.stdout).contains("missing"));

    fs::create_dir(&build).unwrap();
    fs::write(&stamp, "not-a-revision\n").unwrap();
    let malformed = verify_stamp(&fixture);
    assert!(!malformed.status.success());
    assert!(text(&malformed.stdout).contains("malformed"));

    fs::write(&stamp, format!("{}\n", fixture.psoxide_revision)).unwrap();
    let matching = verify_stamp(&fixture);
    assert!(matching.status.success(), "{}", text(&matching.stderr));

    fixture.advance_psoxide();
    let stale = verify_stamp(&fixture);
    assert!(!stale.status.success());
    assert!(text(&stale.stdout).contains("but PSoXide is"));
}

#[test]
fn headless_path_requires_the_program_revision_stamp() {
    let links = Links::new();
    let headless = links.dry_run("quake-headless-check", &[]);
    let stamp_at = index_of(&headless, "programs.psoxide-revision");
    // Was `tools/check_release_chainloads.py`.
    let replay_at = index_of(&headless, "disc-tools chainloads");
    assert!(headless.contains("--target \"QUAKE SHAREWARE\""));
    assert!(stamp_at < replay_at);
}

#[test]
fn cortex_symbol_gate_rejects_missing_empty_and_forbidden_maps() {
    let links = Links::new();
    let directory = tempfile::tempdir().unwrap();
    let link_map = directory.path().join("guest.map");
    let cases: [(Option<&str>, bool); 4] = [
        (None, false),
        (Some(""), false),
        (Some(" 00001000 __udivdi3\n"), false),
        (Some(" 00001000 guest_main\n"), true),
    ];
    for (content, expected_success) in cases {
        if let Some(content) = content {
            fs::write(&link_map, content).unwrap();
        }
        let result = links.make(
            &["cortex-symbol-check"],
            &[("CORTEX_CURRENT_MAP", &link_map.display().to_string())],
        );
        assert_eq!(
            result.status.success(),
            expected_success,
            "{content:?}: {}{}",
            text(&result.stdout),
            text(&result.stderr)
        );
    }
}

#[test]
fn cortex_bakes_only_the_current_project() {
    let makefile = makefile();
    let project =
        read(repo_root().join("games/PSoXide-editor/editor/projects/default/project.ron"));
    let scene = project.split("resources:").next().unwrap();
    assert!(makefile.contains("$(CORTEX_CURRENT_PSOXIDE)/editor/projects/default"));
    assert!(makefile.contains("CORTEX_CURRENT_PROJECT := $(BUILD)/cortex-current-04b"));
    // The pin is whatever games/PSoXide-editor is checked out at;
    // a literal here would turn every routine repin into a test failure.
    let pinned = git(
        &repo_root().join("games/PSoXide-editor"),
        &["rev-parse", "HEAD"],
    );
    assert!(makefile.contains(&format!("CORTEX_CURRENT_EXPECTED_PSOXIDE_REV ?= {pinned}")));
    assert!(makefile.contains("cortex_ignition_tech_demo_0_4b.cue"));
    assert!(!makefile.contains("CORTEX_LEGACY"));
    assert!(!makefile.contains("CORTEX IGNITION LEGACY"));
    assert!(!makefile.contains("$(CORTEX_CURRENT_PSOXIDE)/editor/samples/cortex_v1"));
    assert!(makefile
        .contains("CORTEX_CURRENT_GUEST_STAGE_ROOT ?= /tmp/psoxide-psx-guest-v1-cortex-current"));
    assert!(makefile.contains("PSOXIDE_GUEST_STAGE_ROOT=\"$(CORTEX_CURRENT_GUEST_STAGE_ROOT)\""));
    assert_eq!(
        makefile
            .matches("PSOXIDE_GUEST_CARGO_HOME=\"$(CORTEX_GUEST_CARGO_HOME)\"")
            .count(),
        1
    );
    // These must be live scene entities, not catalogue-only resources.
    assert!(scene.contains("name: \"Aletha (Player)\", kind: Entity"));
    assert!(scene.contains("CharacterController(character: Some((62))"));
    assert!(scene.contains("name: \"Intake Custodian\", kind: Entity"));
    assert!(scene.contains("CharacterController(character: Some((113))"));
    assert!(scene.contains("name: \"Heavy Enemy\", kind: Entity"));
    assert!(scene.contains("CharacterController(character: Some((95))"));
}

#[test]
fn half_life_pressing_is_the_default_disc_plus_half_life() {
    let links = Links::new();
    let default = links.dry_run("disc-only", &[]);
    let half_life = links.dry_run("disc-only", &[("HL", "1")]);
    assert!(half_life.contains("--image \"HALF-LIFE="));
    assert!(!default.contains("--image \"HALF-LIFE="));
    for pressing in [&default, &half_life] {
        assert!(pressing.contains("--image \"CORTEX IGNITION="));
        assert!(!pressing.contains("CORTEX IGNITION LEGACY"));
    }
    // Cortex Ignition is on the carousel of both pressings since 2026-09-03;
    // nothing is gated behind the Konami code any more.
    assert!(!default.contains("--gate \"CORTEX IGNITION\""));
    assert!(!half_life.contains("--gate \"CORTEX IGNITION\""));
    // The carousel order is the argument order. The Makefile comment dated
    // 2026-09-26 reverses it so that Cortex is last before CREDITS, which puts
    // Half-Life before Cortex. (The Python test asserted the old order, Cortex
    // before Half-Life, which cannot hold on this Makefile.)
    let current_at = index_of(&half_life, "--image \"CORTEX IGNITION=");
    let half_life_at = index_of(&half_life, "--image \"HALF-LIFE=");
    assert!(half_life_at < current_at);
    // Everything the default pressing carries, the HL pressing carries too.
    for argument in [
        "--image \"QUAKE SHAREWARE=".to_string(),
        format!("--version-of \"QUAKE SHAREWARE={}\"", pinned_menu_version()),
        "disc-tools quake verify".to_string(),
        "disc-tools quake receipt".to_string(),
    ] {
        assert!(default.contains(&argument), "default: {argument}");
        assert!(half_life.contains(&argument), "HL: {argument}");
    }
    assert!(half_life.contains("PSoXide Demo Disc HL.bin"));
}

#[test]
fn half_life_demo_build_packs_without_installing() {
    let makefile = makefile();
    assert!(makefile.contains("cargo run --release -- pack --psoxide $(PROGRAMS_PSOXIDE)"));
    assert!(!makefile.contains("cargo run --release -- disc --psoxide $(PROGRAMS_PSOXIDE)"));
}

#[test]
fn distribution_targets_publish_only_the_standard_pressing() {
    let makefile = makefile();
    assert!(!makefile.contains("publication-block"));
    // release-web now depends on the host tool binary, so its rule line is
    // `release-web: $(DISC_TOOLS)` (the Python test looked for a bare
    // `release-web:`); itch is unchanged.
    for target in ["release-web: $(DISC_TOOLS)", "itch:"] {
        assert!(makefile.contains(&format!("\n{target}\n")), "{target}");
    }
    // The pressing flags grew WO and HWT since the Python test was written.
    assert!(makefile.contains("\nPRIVATE := $(strip $(HL)$(CS)$(HK)$(WO)$(HWT))\n"));
    assert!(makefile.contains(
        "release-web: $(DISC_TOOLS)\n\t@test -z \"$(PRIVATE)\" || { echo \
         \"release-web: the HL, CS and HK pressings are never distributed\""
    ));
    assert!(makefile.contains(
        "itch:\n\t@test -z \"$(PRIVATE)\" || { echo \
         \"itch: the HL, CS and HK pressings are never distributed\""
    ));
}

/// `make quake-verify` with the pins taken from `fixture`, plus overrides.
fn make_verify(links: &Links, fixture: &QuakeFixture, overrides: &[(&str, &str)]) -> Output {
    let path = |p: &Path| p.display().to_string();
    let mut assignments: Vec<(&str, String)> = vec![
        ("PSOXIDE", path(&fixture.psoxide)),
        ("PROGRAMS_PSOXIDE", path(&fixture.psoxide)),
        (
            "PROGRAMS_EXPECTED_PSOXIDE_REV",
            fixture.psoxide_revision.clone(),
        ),
        ("BUILD", path(fixture.programs_stamp.parent().unwrap())),
        ("QUAKE_SRC", path(&fixture.source)),
        ("QUAKE_EXPECTED_REV", fixture.revision.clone()),
        (
            "QUAKE_EXPECTED_PSOXIDE_REV",
            fixture.psoxide_revision.clone(),
        ),
        (
            "QUAKE_EXPECTED_PROVENANCE_SHA256",
            digest(&fixture.provenance),
        ),
        ("QUAKE_EXPECTED_CUE_SHA256", digest(&fixture.cue)),
        ("QUAKE_EXPECTED_BIN_SHA256", digest(&fixture.bin)),
        ("QUAKE_EXPECTED_EXE_SHA256", digest(&fixture.exe)),
    ];
    for (key, value) in overrides {
        assignments.retain(|(k, _)| k != key);
        assignments.push((key, value.to_string()));
    }
    let borrowed: Vec<(&str, &str)> = assignments.iter().map(|(k, v)| (*k, v.as_str())).collect();
    links.make(&["quake-verify"], &borrowed)
}

#[test]
fn a_correct_payload_passes() {
    let links = Links::new();
    let fixture = QuakeFixture::new();
    let passing = make_verify(&links, &fixture, &[]);
    assert!(
        passing.status.success(),
        "{}{}",
        text(&passing.stdout),
        text(&passing.stderr)
    );
}

#[test]
fn absent_stale_dirty_and_mispinned_payloads_all_fail() {
    type Mutate = fn(&QuakeFixture);
    /// A label, the `make` overrides, what to break in the fixture, and the
    /// text the failure must carry.
    type Case<'a> = (&'a str, Vec<(&'a str, &'a str)>, Option<Mutate>, &'a str);
    let zero_rev = "0".repeat(40);
    let zero_hash = "0".repeat(64);
    let cases: Vec<Case> = vec![
        (
            "absent",
            vec![("QUAKE_SRC", "/nonexistent/quake")],
            None,
            "does not exist",
        ),
        (
            "stale quake pin",
            vec![("QUAKE_EXPECTED_REV", &zero_rev)],
            None,
            "Quake source checkout revision mismatch",
        ),
        (
            "wrong psoxide pin",
            vec![("QUAKE_EXPECTED_PSOXIDE_REV", &zero_rev)],
            None,
            "PSoXide checkout revision mismatch",
        ),
        (
            "stale artifact hash",
            vec![("QUAKE_EXPECTED_BIN_SHA256", &zero_hash)],
            None,
            "bin SHA-256 mismatch",
        ),
        (
            "dirty quake checkout",
            vec![],
            Some(|f| fs::write(f.source.join("tracked.txt"), "changed\n").unwrap()),
            "Quake source checkout is dirty",
        ),
        (
            "dirty psoxide checkout",
            vec![],
            Some(|f| fs::write(f.psoxide.join("sdk.txt"), "changed\n").unwrap()),
            "PSoXide checkout is dirty",
        ),
        (
            "stale ordinary programs",
            vec![],
            Some(|f| fs::write(&f.programs_stamp, format!("{}\n", "0".repeat(40))).unwrap()),
            "ordinary-program SDK revision mismatch",
        ),
        (
            "missing program stamp",
            vec![],
            Some(|f| fs::remove_file(&f.programs_stamp).unwrap()),
            "revision stamp does not exist",
        ),
    ];
    let links = Links::new();
    for (label, overrides, mutate, error) in cases {
        let fixture = QuakeFixture::new();
        if let Some(mutate) = mutate {
            mutate(&fixture);
        }
        let failed = make_verify(&links, &fixture, &overrides);
        assert!(
            !failed.status.success(),
            "{label}: {}",
            text(&failed.stdout)
        );
        assert!(
            text(&failed.stderr).contains(error),
            "{label}: {}",
            text(&failed.stderr)
        );
    }
}

fn make_repin(links: &Links, fixture: &QuakeFixture) -> Output {
    links.make(
        &["quake-repin"],
        &[("QUAKE_SRC", &fixture.source.display().to_string())],
    )
}

#[test]
fn repin_prints_the_pins_a_built_tree_implies() {
    let links = Links::new();
    let fixture = QuakeFixture::new();
    let printed = make_repin(&links, &fixture);
    assert!(printed.status.success(), "{}", text(&printed.stderr));
    let stdout = text(&printed.stdout);
    let expected = [
        ("QUAKE_EXPECTED_REV", fixture.revision.clone()),
        (
            "QUAKE_EXPECTED_PSOXIDE_REV",
            fixture.psoxide_revision.clone(),
        ),
        (
            "QUAKE_EXPECTED_PROVENANCE_SHA256",
            digest(&fixture.provenance),
        ),
        ("QUAKE_EXPECTED_CUE_SHA256", digest(&fixture.cue)),
        ("QUAKE_EXPECTED_BIN_SHA256", digest(&fixture.bin)),
        ("QUAKE_EXPECTED_EXE_SHA256", digest(&fixture.exe)),
    ];
    for (name, value) in expected {
        assert!(stdout.contains(&format!("{name} ?= {value}")), "{name}");
    }
    // Every pin the Makefile holds is a pin the repin prints.
    let pins = Regex::new(r"(?m)^(QUAKE_EXPECTED_\w+) \?=").unwrap();
    for found in pins.captures_iter(&makefile()) {
        assert!(
            stdout.contains(&format!("{} ?= ", &found[1])),
            "{}",
            &found[1]
        );
    }
    assert!(!stdout.contains("WARNING"));
}

#[test]
fn repin_refuses_to_speak_for_a_dirty_tree() {
    let links = Links::new();
    let fixture = QuakeFixture::new();
    fs::write(fixture.source.join("tracked.txt"), "changed\n").unwrap();
    let printed = make_repin(&links, &fixture);
    assert!(printed.status.success(), "{}", text(&printed.stderr));
    assert!(text(&printed.stdout).contains("WARNING: the Quake tree is dirty"));
}

#[test]
fn the_ordinary_check_runs_the_quake_pin_check() {
    let links = Links::new();
    let checked = links.make(&["-n", "check"], &[]);
    assert!(checked.status.success(), "{}", text(&checked.stderr));
    // Was `tools/quake_disc.py verify`.
    assert!(text(&checked.stdout).contains("disc-tools quake verify"));
}

#[test]
fn layout_cannot_start_before_the_payload_is_verified() {
    let links = Links::new();
    let laid_out = links.dry_run("disc-only", &[]);
    let stamp_at = index_of(&laid_out, "programs.psoxide-revision");
    let verify_at = index_of(&laid_out, "disc-tools quake verify");
    let layout_at = index_of(&laid_out, "--image \"QUAKE SHAREWARE=");
    let receipt_at = index_of(&laid_out, "disc-tools quake receipt");
    assert!(stamp_at < verify_at);
    assert!(verify_at < layout_at);
    assert!(layout_at < receipt_at);
}
