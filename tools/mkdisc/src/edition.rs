//! Edition files: which programs a scene disc carries, in carousel order, one section each.
//!
//! Two files feed one pressing. `--programs` holds what stays the same from pressing to pressing
//! (the shelf text and the art id of every program the demo disc has ever carried); `--edition`
//! names the programs of this edition, in carousel order, with what each pressing pins (the
//! image or executable and its version). Adding a game to an edition is one section there.
//!
//! ```text
//! # comment
//! [HALF-LIFE]                       the name the shelf shows (the table's entry name)
//! image    = /path/to/hl-psx.cue    a whole scene disc (or `exe = /path/x.exe`, a bare exe)
//! version  = 0.9                    the version its own source declares
//! describe = English text|Italian text
//! art      = halflife               shelf screenshots: <shots dir>/<art>-1.shot, -2, ...
//! shot     = /path/extra.shot       one more screenshot (repeatable)
//! gate     = /path/psx-gate.toml    the program's own gate file, at its built revision
//! ```
//!
//! A key the edition gives wins over the programs file's. Paths are taken as written (an
//! edition pins files outside this repository: lane builds, library discs).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// One program of an edition, every key it ended up with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub keys: BTreeMap<String, Vec<String>>,
}

impl Entry {
    /// The last value of `key`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.keys
            .get(key)
            .and_then(|v| v.last())
            .map(String::as_str)
    }

    /// Every value of `key`, in file order.
    pub fn all(&self, key: &str) -> &[String] {
        self.keys.get(key).map_or(&[], Vec::as_slice)
    }
}

const KEYS: [&str; 7] = ["image", "exe", "version", "describe", "art", "shot", "gate"];

/// Parse one file into its sections, in file order.
pub fn parse(text: &str, what: &str) -> Result<Vec<Entry>, String> {
    let mut out: Vec<Entry> = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = |msg: String| format!("{what} line {}: {msg}", n + 1);
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let name = name.trim();
            if name.is_empty() || out.iter().any(|e| e.name == name) {
                return Err(at(format!("section [{name}] is empty or given twice")));
            }
            out.push(Entry {
                name: name.to_string(),
                keys: BTreeMap::new(),
            });
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| at("want `key = value` or `[NAME]`".into()))?;
        let (key, value) = (key.trim(), value.trim());
        if !KEYS.contains(&key) {
            return Err(at(format!(
                "unknown key {key:?} (one of {})",
                KEYS.join(", ")
            )));
        }
        let entry = out
            .last_mut()
            .ok_or_else(|| at("a key before the first [NAME]".into()))?;
        entry
            .keys
            .entry(key.to_string())
            .or_default()
            .push(value.to_string());
    }
    Ok(out)
}

/// The edition's programs in its order, each with the programs file's keys under its own.
pub fn merge(programs: &[Entry], edition: &[Entry]) -> Result<Vec<Entry>, String> {
    let mut out = Vec::with_capacity(edition.len());
    for entry in edition {
        let mut merged = programs
            .iter()
            .find(|p| p.name == entry.name)
            .cloned()
            .unwrap_or_else(|| Entry {
                name: entry.name.clone(),
                keys: BTreeMap::new(),
            });
        for (key, values) in &entry.keys {
            merged.keys.insert(key.clone(), values.clone());
        }
        match (merged.get("image"), merged.get("exe")) {
            (Some(_), None) | (None, Some(_)) => {}
            _ => {
                return Err(format!(
                    "[{}]: give exactly one of `image =` (a scene disc) and `exe =` (a bare exe)",
                    entry.name
                ))
            }
        }
        out.push(merged);
    }
    Ok(out)
}

/// Read and merge `--programs` (optional) and `--edition`.
pub fn load(programs: Option<&Path>, edition: &Path) -> Result<Vec<Entry>, String> {
    let read = |p: &Path| fs::read_to_string(p).map_err(|e| format!("read {}: {e}", p.display()));
    let shared = match programs {
        Some(p) => parse(&read(p)?, &p.display().to_string())?,
        None => Vec::new(),
    };
    let own = parse(&read(edition)?, &edition.display().to_string())?;
    merge(&shared, &own)
}

/// The cooked screenshots of art id `art` in `dir`: `<art>-1.shot`, `-2`, ... while they exist.
pub fn art_shots(dir: &Path, art: &str) -> Vec<PathBuf> {
    (1..)
        .map(|k| dir.join(format!("{art}-{k}.shot")))
        .take_while(|p| p.is_file())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROGRAMS: &str = "\
[HALF-LIFE]
describe = English|Italiano
art = halflife
[QD UP]
describe = Pool|Biliardo
";

    #[test]
    fn the_edition_orders_and_overrides() {
        let programs = parse(PROGRAMS, "programs").unwrap();
        let edition = parse(
            "[QD UP]\nimage = q.cue\nversion = 1\n[HALF-LIFE]\nimage = h.cue\nart = other\n",
            "edition",
        )
        .unwrap();
        let merged = merge(&programs, &edition).unwrap();
        assert_eq!(merged[0].name, "QD UP");
        assert_eq!(merged[0].get("describe"), Some("Pool|Biliardo"));
        assert_eq!(merged[1].get("art"), Some("other"));
        assert_eq!(merged[1].get("describe"), Some("English|Italiano"));
    }

    #[test]
    fn a_program_needs_exactly_one_source() {
        let edition = parse("[X]\nversion = 1\n", "e").unwrap();
        assert!(merge(&[], &edition).is_err());
        let edition = parse("[X]\nimage = a\nexe = b\n", "e").unwrap();
        assert!(merge(&[], &edition).is_err());
    }

    #[test]
    fn unknown_keys_and_orphan_keys_are_refused() {
        assert!(parse("[X]\ncolour = red\n", "e").is_err());
        assert!(parse("image = a\n", "e").is_err());
        assert!(parse("[X]\n[X]\n", "e").is_err());
    }
}
