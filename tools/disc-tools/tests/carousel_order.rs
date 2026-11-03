//! The carousel order both editions share.
//!
//! The pressed order lives in the Makefile's `MKDISC_ARGS`, which the public
//! and the private lineup pressings both lay out (a lineup JSON names inputs,
//! not positions). PSXcel moved from first to last in 48e6a33, and a private
//! pressing laid out before that kept the old order until it was re-pressed.
//! These tests read the Makefile and the private lineup file, so a move in one
//! place that misses the other fails here instead of on a console.

use regex::Regex;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: PathBuf) -> String {
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The entry names `MKDISC_ARGS` lays out, in carousel order, with every
/// optional `*_ARGS` block expanded as if its flag were set.
fn pressed_order() -> Vec<String> {
    let makefile = read(repo_root().join("Makefile"));
    let block_name = Regex::new(r#"(?m)^([A-Z_]+_ARGS) = --(?:game|image) "([^=]+)="#).unwrap();
    let optional: Vec<(String, String)> = block_name
        .captures_iter(&makefile)
        .map(|c| (c[1].to_string(), c[2].to_string()))
        .collect();

    let start = makefile
        .find("MKDISC_ARGS = ")
        .expect("Makefile defines MKDISC_ARGS");
    let mut block = String::new();
    for line in makefile[start..].lines() {
        block.push_str(line);
        block.push('\n');
        if !line.trim_end().ends_with('\\') {
            break;
        }
    }

    let token = Regex::new(r#"--(?:game|image) "([^=]+)=|\$\(([A-Z_]+_ARGS)\)"#).unwrap();
    let mut order = Vec::new();
    for c in token.captures_iter(&block) {
        if let Some(name) = c.get(1) {
            order.push(name.as_str().to_string());
        } else {
            let args = &c[2];
            if let Some((_, name)) = optional.iter().find(|(n, _)| n == args) {
                order.push(name.clone());
            }
        }
    }
    order
}

#[test]
fn psxcel_is_last_before_credits_on_every_pressing() {
    let order = pressed_order();
    assert_eq!(order.last().map(String::as_str), Some("PSXCEL"), "{order:?}");
    assert_eq!(
        order.iter().filter(|n| n.as_str() == "PSXCEL").count(),
        1,
        "{order:?}"
    );
}

#[test]
fn the_standard_order_is_the_documented_one() {
    let order = pressed_order();
    let standard: Vec<&str> = order
        .iter()
        .map(String::as_str)
        .filter(|n| {
            matches!(
                *n,
                "PSOXIDE ARCADE"
                    | "CELESTE COLLECTION"
                    | "NITROXIDE"
                    | "VOXIDE"
                    | "QUAKE SHAREWARE"
                    | "CORTEX IGNITION"
                    | "PSXCEL"
            )
        })
        .collect();
    assert_eq!(
        standard,
        [
            "PSOXIDE ARCADE",
            "CELESTE COLLECTION",
            "NITROXIDE",
            "VOXIDE",
            "QUAKE SHAREWARE",
            "CORTEX IGNITION",
            "PSXCEL",
        ]
    );
}

/// Both Cortex engines are on the Private shelf, the old one first: Cortex
/// Ignition 0.4b (both editions), then the Cortex Citadel new-engine test.
#[test]
fn cortex_citadel_follows_cortex_ignition() {
    let order = pressed_order();
    let at = |name: &str| order.iter().position(|n| n == name);
    let ignition = at("CORTEX IGNITION").expect("Cortex Ignition is pressed");
    assert_eq!(at("CORTEX CITADEL"), Some(ignition + 1), "{order:?}");
}

/// The Citadel test is Private only: its Makefile block is chosen by a flag
/// that the Private channel sets and the Public channel does not.
#[test]
fn cortex_citadel_is_pressed_by_the_private_channel_only() {
    let makefile = read(repo_root().join("Makefile"));
    let flags = makefile
        .lines()
        .find(|l| l.starts_with("PRIVATE_PRESS_FLAGS"))
        .expect("the Private channel's flags");
    assert!(flags.split_whitespace().any(|f| f == "CITADEL=1"), "{flags}");
    let public: Vec<&str> = makefile
        .lines()
        .skip_while(|l| !l.starts_with("press-public:"))
        .take_while(|l| !l.starts_with("press-private:"))
        .collect();
    assert!(!public.is_empty() && !public.iter().any(|l| l.contains("CITADEL")), "{public:?}");
    // A flagged pressing is Private (the launcher gets its `private` feature).
    assert!(
        makefile.contains("PRIVATE := $(strip $(HL)$(CS)$(HK)$(WO)$(HWT)$(OOT)$(CITADEL))"),
        "CITADEL must make a pressing Private"
    );
}

/// No Public lineup file names the new engine, and each Public lineup's Cortex
/// row is a Cortex Ignition 0.4b image (the gate itself is `lineup check`).
#[test]
fn no_public_lineup_names_cortex_citadel() {
    let release = repo_root().join("release");
    for entry in fs::read_dir(&release).expect("release/ exists") {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !(name.starts_with("lineup-") && name.ends_with(".json")) || name.contains("private") {
            continue;
        }
        let text = read(path);
        assert!(!text.to_uppercase().contains("CITADEL"), "{name} names the new engine");
    }
}

/// Every private lineup lists its programs in the order they are pressed, so
/// the file reads like the carousel it produces.
#[test]
fn private_lineups_list_programs_in_carousel_order() {
    let order = pressed_order();
    let release = repo_root().join("release");
    let mut checked = 0;
    for entry in fs::read_dir(&release).expect("release/ exists") {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !(name.starts_with("lineup-") && name.ends_with("-private.json")) {
            continue;
        }
        let lineup: Value = serde_json::from_str(&read(path.clone())).unwrap();
        let names: Vec<&str> = lineup["programs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        let mut expected: Vec<&str> = order
            .iter()
            .map(String::as_str)
            .filter(|n| names.contains(n))
            .collect();
        // A lineup may carry fewer programs than the Makefile can press.
        assert!(!expected.is_empty(), "{name}: no program is pressed");
        assert_eq!(names, expected, "{name} is not in carousel order");
        expected.clear();
        checked += 1;
    }
    assert!(checked > 0, "no private lineup in release/");
}
