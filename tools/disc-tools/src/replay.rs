//! What the headless gates share: running the frontend with stdout and stderr
//! merged, reading its CSV logs the way Python's `csv.DictReader` did, the
//! Python-compatible integer and line parsing the logs need, and a scratch
//! directory that cleans up after itself.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

use crate::util::{Error, Result};

/// Python's `str.splitlines()`: every line break the standard library
/// honours, with the break itself dropped (`\r\n` is one break).
pub(crate) fn splitlines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if matches!(
            c,
            '\n' | '\r'
                | '\x0b'
                | '\x0c'
                | '\x1c'
                | '\x1d'
                | '\x1e'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        ) {
            lines.push(&text[start..at]);
            start = at + c.len_utf8();
            if c == '\r' && chars.peek().is_some_and(|(_, next)| *next == '\n') {
                chars.next();
                start += 1;
            }
        }
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

/// The emulator's stdout without its `[cli]` housekeeping lines (paths and
/// timings that legitimately differ between two runs).
pub(crate) fn stdout_core(stdout: &str) -> String {
    splitlines(stdout)
        .into_iter()
        .filter(|line| !line.starts_with("[cli] "))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn pattern(cell: &'static OnceLock<Regex>, text: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(text).expect("static regex"))
}

pub(crate) fn number(text: &str, radix: u32) -> Result<u64> {
    u64::from_str_radix(text, radix)
        .map_err(|e| Error(format!("bad number {text:?} in emulator output: {e}")))
}

/// A CSV log read the way Python's `csv.DictReader` reads it: the first
/// record names the columns, blank lines are skipped, quoted fields may hold
/// commas, quotes (doubled) and line breaks.
pub(crate) struct Csv {
    pub(crate) header: Vec<String>,
    pub(crate) rows: Vec<Vec<String>>,
}

impl Csv {
    pub(crate) fn read(path: &Path) -> Result<Csv> {
        let bytes = fs::read(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        ensure!(bytes.is_ascii(), "{}: not ASCII", path.display());
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let mut records: Vec<Vec<String>> = Vec::new();
        let (mut record, mut field) = (Vec::new(), String::new());
        let (mut quoted, mut started) = (false, false);
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '"' if quoted && chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' if !quoted && field.is_empty() => {
                    quoted = true;
                    started = true;
                }
                '"' if quoted => quoted = false,
                ',' if !quoted => {
                    record.push(std::mem::take(&mut field));
                    started = true;
                }
                '\r' | '\n' if !quoted => {
                    if c == '\r' && chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    if started || !field.is_empty() {
                        record.push(std::mem::take(&mut field));
                        records.push(std::mem::take(&mut record));
                    }
                    started = false;
                }
                _ => {
                    field.push(c);
                    started = true;
                }
            }
        }
        if started || !field.is_empty() {
            record.push(field);
            records.push(record);
        }
        let mut records = records.into_iter();
        let header = records.next().unwrap_or_default();
        Ok(Csv {
            header,
            rows: records.collect(),
        })
    }

    /// The cell of `row` under column `name`: an error if the log has no such
    /// column, `None` if this row is too short to reach it.
    pub(crate) fn cell<'a>(&self, row: &'a [String], name: &str) -> Result<Option<&'a str>> {
        let Some(column) = self.header.iter().rposition(|h| h == name) else {
            bail!("log has no column {name:?}");
        };
        Ok(row.get(column).map(String::as_str))
    }

    /// A cell that must be there, for the numeric columns.
    pub(crate) fn required<'a>(&self, row: &'a [String], name: &str) -> Result<&'a str> {
        self.cell(row, name)?
            .ok_or_else(|| Error(format!("log row has no value for column {name:?}")))
    }
}

/// Python's `int(text, radix)` for the plain forms the logs contain.
pub(crate) fn parse_int(text: &str, radix: u32) -> Result<u64> {
    let trimmed = text.trim().trim_start_matches('+');
    let digits = if radix == 16 {
        trimmed
            .strip_prefix("0x")
            .or_else(|| trimmed.strip_prefix("0X"))
            .unwrap_or(trimmed)
    } else {
        trimmed
    };
    u64::from_str_radix(digits, radix)
        .map_err(|_| Error(format!("invalid literal for int(): {text:?}")))
}

/// One BCD byte, written as hex text, to its value.
pub(crate) fn bcd(value: &str) -> Result<u64> {
    let raw = parse_int(value, 16)?;
    let (high, low) = (raw >> 4, raw & 0x0F);
    ensure!(high <= 9 && low <= 9, "invalid BCD byte {value}");
    Ok(high * 10 + low)
}

/// The LBA a SetLoc (command 0x02) row seeks to, or `None` for any other row.
pub(crate) fn setloc_lba(csv: &Csv, row: &[String]) -> Result<Option<i64>> {
    if csv.cell(row, "command")? != Some("0x02")
        || parse_int(csv.required(row, "param_len")?, 10)? != 3
    {
        return Ok(None);
    }
    let parts: Vec<u64> = csv
        .required(row, "params")?
        .split_whitespace()
        .map(bcd)
        .collect::<Result<Vec<_>>>()?;
    let [minute, second, frame] = parts[..] else {
        bail!("SetLoc row does not carry three BCD bytes");
    };
    Ok(Some(((minute * 60 + second) * 75 + frame) as i64 - 150))
}

/// A scratch directory under the system temp dir, removed when dropped.
pub(crate) struct Scratch(pub(crate) PathBuf);

impl Scratch {
    pub(crate) fn new(prefix: &str) -> Result<Scratch> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{stamp}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Scratch(path))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Python's `shlex.quote`.
fn shlex_quote(word: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "@%+=:,./-_".contains(c);
    if !word.is_empty() && word.chars().all(safe) {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', "'\"'\"'"))
}

/// Python's `shlex.join`.
pub(crate) fn shlex_join(words: &[String]) -> String {
    words
        .iter()
        .map(|word| shlex_quote(word))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Python's `shlex.split` for one line: whitespace separates words, single
/// quotes are literal, double quotes honour backslash escapes of `"` and `\`,
/// and a bare backslash escapes the next character. `#` comments are not
/// recognised (`shlex.split` defaults to `comments=False`).
pub(crate) fn shlex_split(line: &str) -> Result<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\r' | '\n' => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(inner) => word.push(inner),
                        None => bail!("No closing quotation"),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(escaped @ ('"' | '\\')) => word.push(escaped),
                            Some(other) => {
                                word.push('\\');
                                word.push(other);
                            }
                            None => bail!("No closing quotation"),
                        },
                        Some(inner) => word.push(inner),
                        None => bail!("No closing quotation"),
                    }
                }
            }
            '\\' => {
                in_word = true;
                match chars.next() {
                    Some(escaped) => word.push(escaped),
                    None => bail!("No escaped character"),
                }
            }
            other => {
                in_word = true;
                word.push(other);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Ok(words)
}

/// Run `command` to completion with stdout and stderr merged into one file
/// (their interleaving is kept, as Python's single pipe kept it) and return
/// the exit code (124 when `timeout` expires and the child is killed) with
/// the text.
pub(crate) fn run_merged(
    command: &mut std::process::Command,
    output_path: &Path,
    timeout: Option<std::time::Duration>,
) -> Result<(i32, String)> {
    use std::process::Stdio;
    let output = fs::File::create(output_path)?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::from(output))
        .spawn()
        .map_err(|e| Error(format!("cannot run {:?}: {e}", command.get_program())))?;
    let started = std::time::Instant::now();
    let code = loop {
        if let Some(status) = child.try_wait()? {
            // A signal death has no code; treat it as a failure.
            break status.code().unwrap_or(-1);
        }
        if timeout.is_some_and(|limit| started.elapsed() >= limit) {
            let _ = child.kill();
            let _ = child.wait();
            break 124;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let text = String::from_utf8_lossy(&fs::read(output_path)?).into_owned();
    Ok((code, text))
}

/// A binary PPM (`P6`, maxval 255) as the frontend dumps it.
pub(crate) struct Ppm {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) pixels: Vec<u8>,
}

impl Ppm {
    /// Header tokens are whitespace-separated; the single whitespace byte after
    /// the maximum value starts the pixel data.
    pub(crate) fn parse(data: &[u8]) -> Option<Ppm> {
        let mut at = 0;
        let token = |at: &mut usize| -> Option<&[u8]> {
            while *at < data.len() && data[*at].is_ascii_whitespace() {
                *at += 1;
            }
            let start = *at;
            while *at < data.len() && !data[*at].is_ascii_whitespace() {
                *at += 1;
            }
            (*at > start).then(|| &data[start..*at])
        };
        if token(&mut at)? != b"P6" {
            return None;
        }
        let number = |t: &[u8]| std::str::from_utf8(t).ok()?.parse::<usize>().ok();
        let width = number(token(&mut at)?)?;
        let height = number(token(&mut at)?)?;
        if number(token(&mut at)?)? != 255 {
            return None;
        }
        // Exactly one whitespace byte separates the header from the pixels.
        let pixels = data.get(at + 1..)?;
        (pixels.len() == width * height * 3).then(|| Ppm {
            width,
            height,
            pixels: pixels.to_vec(),
        })
    }

    pub(crate) fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        let at = (y * self.width + x) * 3;
        [self.pixels[at], self.pixels[at + 1], self.pixels[at + 2]]
    }
}

/// The part of one carousel row the headless gates read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) name: String,
    pub(crate) exe_lba: u32,
    pub(crate) image_lba: u32,
    pub(crate) cdda_track_base: u32,
    pub(crate) payload_fnv: u32,
    pub(crate) version: String,
    pub(crate) flags: u32,
}

impl Row {
    #[cfg(test)]
    pub(crate) fn hidden(&self) -> bool {
        self.flags & crate::disc::FLAG_HIDDEN != 0
    }
}

/// The demo table's rows, reading only the fields the gates use (no checks on
/// the description text, and duplicate names are the caller's business).
pub(crate) fn toc_rows(toc: &[u8]) -> Result<Vec<Row>> {
    use crate::disc::{le32, text_field};
    use crate::disc::{TOC_ENTRY_BYTES, TOC_FLAGS_AT, TOC_HEADER_BYTES, TOC_NAME_BYTES};
    use crate::disc::{TOC_VERSION_AT, TOC_VERSION_BYTES};
    ensure!(
        toc.len() == crate::disc::TOC_SECTORS as usize * crate::disc::USER_DATA_BYTES
            && &toc[..8] == crate::disc::TOC_MAGIC,
        "demo table is absent or malformed"
    );
    let count = le32(toc, 8) as usize;
    let maximum = (toc.len() - TOC_HEADER_BYTES) / TOC_ENTRY_BYTES;
    ensure!(
        count != 0 && count <= maximum,
        "invalid demo table entry count {count}"
    );
    (0..count)
        .map(|index| {
            let at = TOC_HEADER_BYTES + index * TOC_ENTRY_BYTES;
            let row = &toc[at..at + TOC_ENTRY_BYTES];
            Ok(Row {
                name: text_field(&row[..TOC_NAME_BYTES])?,
                exe_lba: le32(row, 24),
                image_lba: le32(row, 28),
                cdda_track_base: le32(row, 32),
                payload_fnv: le32(row, 36),
                version: text_field(&row[TOC_VERSION_AT..TOC_VERSION_AT + TOC_VERSION_BYTES])?,
                flags: le32(row, TOC_FLAGS_AT),
            })
        })
        .collect()
}
