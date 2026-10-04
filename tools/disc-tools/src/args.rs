//! A deliberately small argument reader: `--name value`, `--name=value`,
//! repeatable options, boolean flags and positionals. `mkdisc` parses its own
//! arguments by hand for the same reason: no dependency for a few dozen lines.

use crate::util::{Error, Result};
use std::path::PathBuf;

#[derive(Debug, Default)]
pub struct Args {
    values: Vec<(String, String)>,
    flags: Vec<String>,
    pub positional: Vec<String>,
}

impl Args {
    /// `values` are the options that take a value, `flags` the ones that do
    /// not. Anything else starting with `--` is an error, as with argparse.
    pub fn parse(raw: &[String], values: &[&str], flags: &[&str]) -> Result<Args> {
        let mut out = Args::default();
        let mut i = 0;
        while i < raw.len() {
            let arg = &raw[i];
            i += 1;
            if arg == "--" {
                out.positional.extend(raw[i..].iter().cloned());
                break;
            }
            let Some(body) = arg.strip_prefix("--") else {
                out.positional.push(arg.clone());
                continue;
            };
            let (name, inline) = match body.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (body, None),
            };
            if flags.contains(&name) {
                if inline.is_some() {
                    return Err(Error(format!("--{name} takes no value")));
                }
                out.flags.push(name.to_string());
            } else if values.contains(&name) {
                let value = match inline {
                    Some(value) => value,
                    None => {
                        let value = raw
                            .get(i)
                            .ok_or_else(|| Error(format!("--{name} needs a value")))?
                            .clone();
                        i += 1;
                        value
                    }
                };
                out.values.push((name.to_string(), value));
            } else {
                return Err(Error(format!("unrecognized argument: {arg}")));
            }
        }
        Ok(out)
    }

    pub fn has(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }

    /// The last value given, like argparse.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn all(&self, name: &str) -> Vec<&str> {
        self.values
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    pub fn require(&self, name: &str) -> Result<&str> {
        self.get(name)
            .ok_or_else(|| Error(format!("--{name} is required")))
    }

    pub fn path(&self, name: &str) -> Option<PathBuf> {
        self.get(name).map(PathBuf::from)
    }

    pub fn require_path(&self, name: &str) -> Result<PathBuf> {
        self.require(name).map(PathBuf::from)
    }

    pub fn int(&self, name: &str) -> Result<Option<i64>> {
        match self.get(name) {
            None => Ok(None),
            Some(text) => text
                .parse::<i64>()
                .map(Some)
                .map_err(|_| Error(format!("--{name} must be an integer, got {text:?}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn reads_values_flags_and_repeats() {
        let args = Args::parse(
            &raw(&[
                "--cue",
                "a.cue",
                "--target=Q",
                "--target",
                "H",
                "--sealed",
                "pos",
            ]),
            &["cue", "target"],
            &["sealed"],
        )
        .unwrap();
        assert_eq!(args.get("cue"), Some("a.cue"));
        assert_eq!(args.all("target"), vec!["Q", "H"]);
        assert!(args.has("sealed"));
        assert_eq!(args.positional, vec!["pos".to_string()]);
    }

    #[test]
    fn rejects_unknown_and_missing_values() {
        assert!(Args::parse(&raw(&["--nope"]), &[], &[]).is_err());
        assert!(Args::parse(&raw(&["--cue"]), &["cue"], &[]).is_err());
    }
}
