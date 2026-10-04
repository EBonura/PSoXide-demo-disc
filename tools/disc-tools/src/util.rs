//! Small helpers every subcommand shares: errors, hashing, git, and a JSON
//! writer that prints exactly what Python's `json.dumps(indent=N)` printed, so
//! receipts written here stay byte-comparable with the ones written before.

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Every failure is a message for a human; the tools exit nonzero and print it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

macro_rules! impl_from {
    ($($ty:ty),* $(,)?) => {$(
        impl From<$ty> for Error {
            fn from(error: $ty) -> Self {
                Error(error.to_string())
            }
        }
    )*};
}
impl_from!(
    std::io::Error,
    serde_json::Error,
    std::str::Utf8Error,
    std::string::FromUtf8Error,
    std::num::ParseIntError,
    regex::Error,
);

/// `bail!("text {}", x)` returns `Err(Error)` from the enclosing function.
#[macro_export]
macro_rules! bail {
    ($($arg:tt)*) => {
        return Err($crate::util::Error(format!($($arg)*)))
    };
}

/// `ensure!(cond, "text {}", x)` is `if !cond { bail!(...) }`.
#[macro_export]
macro_rules! ensure {
    ($cond:expr, $($arg:tt)*) => {
        if !($cond) {
            $crate::bail!($($arg)*);
        }
    };
}

pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn sha256_bytes(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut stream = fs::File::open(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    let mut digest = Sha256::new();
    let mut chunk = vec![0u8; 1 << 20];
    loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        digest.update(&chunk[..n]);
    }
    Ok(hex(&digest.finalize()))
}

pub fn fnv1a32(data: &[u8]) -> u32 {
    let mut digest: u32 = 0x811C_9DC5;
    for &byte in data {
        digest = (digest ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    digest
}

/// `Path::canonicalize`, but with the path in the message (Python's
/// `resolve(strict=True)`).
pub fn resolve(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .map_err(|e| Error(format!("{}: {e}", path.display())))
}

/// `Path::canonicalize` that tolerates a path which does not exist yet, like
/// Python's non-strict `resolve()`: the deepest existing ancestor is resolved
/// and the rest is appended.
pub fn resolve_lenient(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|c| c.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut tail = Vec::new();
    let mut head = absolute.as_path();
    loop {
        if let Ok(real) = head.canonicalize() {
            let mut out = real;
            for part in tail.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (head.file_name(), head.parent()) {
            (Some(name), Some(parent)) => {
                tail.push(name.to_os_string());
                head = parent;
            }
            _ => return absolute,
        }
    }
}

/// `git -C <dir> <args>`, stdout trimmed, nonzero exit is an error.
pub fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let result = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Error(format!("cannot run git: {e}")))?;
    if !result.status.success() {
        bail!(
            "git {} failed for {}: {}",
            args.join(" "),
            dir.display(),
            String::from_utf8_lossy(&result.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&result.stdout).trim().to_string())
}

/// `git -C <dir> <args>` returning raw stdout bytes (for `-z` listings).
pub fn git_bytes(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let result = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| Error(format!("cannot run git: {e}")))?;
    if !result.status.success() {
        bail!(
            "git {} failed for {}: {}",
            args.join(" "),
            dir.display(),
            String::from_utf8_lossy(&result.stderr).trim()
        );
    }
    Ok(result.stdout)
}

/// stdout of a command, trimmed; the command must exit zero.
pub fn command_output(program: &str, args: &[&str], dir: Option<&Path>) -> Result<String> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null());
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let result = command
        .output()
        .map_err(|e| Error(format!("cannot run {program}: {e}")))?;
    if !result.status.success() {
        bail!(
            "{program} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&result.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&result.stdout).trim().to_string())
}

fn write_json_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7E => {
                // ensure_ascii: UTF-16 surrogate pairs above the BMP.
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_value(out: &mut String, value: &Value, indent: usize, sort_keys: bool, level: usize) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => write_json_string(out, s),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('\n');
                out.push_str(&" ".repeat(indent * (level + 1)));
                write_value(out, item, indent, sort_keys, level + 1);
            }
            out.push('\n');
            out.push_str(&" ".repeat(indent * level));
            out.push(']');
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            if sort_keys {
                entries.sort_by(|a, b| a.0.cmp(b.0));
            }
            out.push('{');
            for (i, (key, item)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('\n');
                out.push_str(&" ".repeat(indent * (level + 1)));
                write_json_string(out, key);
                out.push_str(": ");
                write_value(out, item, indent, sort_keys, level + 1);
            }
            out.push('\n');
            out.push_str(&" ".repeat(indent * level));
            out.push('}');
        }
    }
}

/// Python's `json.dumps(value, indent=indent, sort_keys=sort_keys)`
/// (`ensure_ascii` on, `, ` and `: ` separators collapsed to `,` + newline).
pub fn dumps(value: &Value, indent: usize, sort_keys: bool) -> String {
    let mut out = String::new();
    write_value(&mut out, value, indent, sort_keys, 0);
    out
}

/// Write `text` to `path` through a `.tmp` sibling and a rename, creating the
/// parent directory, so a crash never leaves a half-written receipt.
pub fn write_atomic(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    let temporary = path.with_file_name(name);
    fs::write(&temporary, text)?;
    fs::rename(&temporary, path)?;
    Ok(())
}

/// A receipt: `json.dumps(document, indent=2, sort_keys=True) + "\n"`.
pub fn write_receipt(path: &Path, document: &Value) -> Result<()> {
    write_atomic(path, &(dumps(document, 2, true) + "\n"))
}

pub fn read_json(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    serde_json::from_str(&text).map_err(|e| Error(format!("{}: {e}", path.display())))
}

/// `{"path": <resolved>, "bytes": N, "sha256": H}`.
pub fn file_record(path: &Path) -> Result<Value> {
    let path = resolve(path)?;
    ensure!(path.is_file(), "not a regular file: {}", path.display());
    let bytes = fs::metadata(&path)?.len();
    let mut map = serde_json::Map::new();
    map.insert("path".into(), Value::String(path.display().to_string()));
    map.insert("bytes".into(), Value::from(bytes));
    map.insert("sha256".into(), Value::String(sha256_file(&path)?));
    Ok(Value::Object(map))
}

pub fn is_hex(text: &str, length: usize) -> bool {
    text.len() == length
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn is_revision(text: &str) -> bool {
    is_hex(text, 40)
}

pub fn is_digest(text: &str) -> bool {
    is_hex(text, 64)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dumps_matches_python_layout() {
        let value = serde_json::json!({"b": [1, 2, {}], "a": "é\u{1F600}\n", "c": []});
        assert_eq!(
            dumps(&value, 2, true),
            "{\n  \"a\": \"\\u00e9\\ud83d\\ude00\\n\",\n  \"b\": [\n    1,\n    2,\n    {}\n  ],\n  \"c\": []\n}"
        );
        let unsorted = dumps(&value, 1, false);
        assert!(unsorted.starts_with("{\n \"b\": [\n  1,"));
    }

    #[test]
    fn fnv_matches_known_vectors() {
        assert_eq!(fnv1a32(b""), 0x811C_9DC5);
        assert_eq!(fnv1a32(b"a"), 0xE40C_292C);
    }

    #[test]
    fn hex_checks() {
        assert!(is_revision(&"a".repeat(40)));
        assert!(!is_revision(&"A".repeat(40)));
        assert!(!is_digest(&"a".repeat(63)));
    }
}

/// The disc repository this binary belongs to: the nearest ancestor of the
/// executable that holds `release-components.json`, else of the working
/// directory. The scripts used their own location for this; a binary has to
/// look for it, and must not mistake whichever repo it happens to run in.
pub fn repo_root() -> Result<PathBuf> {
    let marker = "release-components.json";
    let mut starts = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        starts.push(exe);
    }
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    for start in starts {
        for ancestor in start.ancestors().skip(1) {
            if ancestor.join(marker).is_file() && ancestor.join("Makefile").is_file() {
                return Ok(ancestor.to_path_buf());
            }
        }
    }
    bail!("cannot find the disc repository root (no {marker} above the executable or the working directory)")
}

/// `os.path.expanduser` for the forms the lineup files use: a leading `~/`.
pub fn expand_home(value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    if value == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    PathBuf::from(value)
}
