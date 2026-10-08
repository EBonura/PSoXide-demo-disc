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
    let block_name = Regex::new(r#"(?m)^([A-Z_]+_ARGS) = --image "([^=]+)="#).unwrap();
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
