//! Strict JSON reading for the provenance sidecar. The sidecar is the root of
//! trust for what the pressing carries, so a key written twice is refused
//! instead of silently keeping the last one the way a plain parse does.

use std::collections::HashSet;

use serde_json::Value;

use crate::util::{Error, Result};

/// One open container while scanning: an object remembers the keys it has
/// seen, and whether the next string it meets is a key or a value.
enum Frame {
    Object {
        keys: HashSet<String>,
        expect_key: bool,
    },
    Array,
}

/// Parse `text` as one JSON document. The outer error is a repeated key (a
/// verdict about the file); the inner one is a syntax error for the caller to
/// word in its own way.
///
/// serde_json collapses repeated keys without a word, so the text is parsed
/// first and then scanned once more for them. The scan can lean on the text
/// being valid JSON: it only has to find the strings that sit in key position.
pub fn parse_without_duplicate_keys(text: &str) -> Result<std::result::Result<Value, String>> {
    let value = match serde_json::from_str::<Value>(text) {
        Ok(value) => value,
        Err(error) => return Ok(Err(error.to_string())),
    };
    if let Some(key) = first_duplicate_key(text)? {
        return Err(Error(format!(
            "provenance JSON contains duplicate key {key:?}"
        )));
    }
    Ok(Ok(value))
}

/// The first key that an object repeats, if any, in document order.
fn first_duplicate_key(text: &str) -> Result<Option<String>> {
    let bytes = text.as_bytes();
    let mut stack: Vec<Frame> = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'{' => stack.push(Frame::Object {
                keys: HashSet::new(),
                expect_key: true,
            }),
            b'[' => stack.push(Frame::Array),
            b'}' | b']' => {
                stack.pop();
            }
            b',' => {
                if let Some(Frame::Object { expect_key, .. }) = stack.last_mut() {
                    *expect_key = true;
                }
            }
            b':' => {
                if let Some(Frame::Object { expect_key, .. }) = stack.last_mut() {
                    *expect_key = false;
                }
            }
            b'"' => {
                let start = at;
                at += 1;
                while bytes[at] != b'"' {
                    // A backslash protects the next byte, including a quote.
                    at += if bytes[at] == b'\\' { 2 } else { 1 };
                }
                if let Some(Frame::Object {
                    keys,
                    expect_key: true,
                }) = stack.last_mut()
                {
                    // Decode the token so "a" and "a" count as one key.
                    let key: String = serde_json::from_str(&text[start..=at])?;
                    if !keys.insert(key.clone()) {
                        return Ok(Some(key));
                    }
                }
            }
            _ => {}
        }
        at += 1;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn duplicate(text: &str) -> Option<String> {
        parse_without_duplicate_keys(text)
            .err()
            .map(|e| e.to_string())
    }

    #[test]
    fn a_repeated_key_is_refused_at_any_depth() {
        let top = duplicate("{\"schema\": 1, \"schema\": 1}").unwrap();
        assert_eq!(top, "provenance JSON contains duplicate key \"schema\"");
        assert!(duplicate("{\"a\": {\"b\": 1, \"c\": 2, \"b\": 3}}")
            .unwrap()
            .contains("\"b\""));
        assert!(duplicate("{\"a\": [{\"k\": 1}, {\"k\": 1, \"k\": 2}]}")
            .unwrap()
            .contains("\"k\""));
    }

    #[test]
    fn an_escaped_spelling_of_a_key_is_the_same_key() {
        assert!(duplicate("{\"a\": 1, \"\\u0061\": 2}").is_some());
        assert!(duplicate("{\"a\\\"b\": 1, \"a\\\"b\": 2}").is_some());
    }

    #[test]
    fn legitimate_repeats_are_not_duplicates() {
        // The same key in sibling objects, a value equal to a key, strings
        // holding braces, quotes and colons, and nested arrays.
        let text = r#"{"a": {"k": 1}, "b": {"k": 2}, "c": "a", "d": "x\",\"a\":", "e": [[{"k": 1}], {"k": 2}], "f": "{}"}"#;
        let parsed = parse_without_duplicate_keys(text).unwrap().unwrap();
        assert_eq!(parsed["b"]["k"], 2);
        assert_eq!(parsed["d"], "x\",\"a\":");
        assert!(duplicate("{}").is_none());
        assert!(duplicate("[]").is_none());
    }

    #[test]
    fn a_syntax_error_is_reported_apart_from_a_duplicate() {
        let parsed = parse_without_duplicate_keys("{").unwrap();
        assert!(parsed.is_err());
        // Invalid text is a parse failure even when it also repeats a key.
        assert!(parse_without_duplicate_keys("{\"a\": 1, \"a\": 2,")
            .unwrap()
            .is_err());
        assert!(parse_without_duplicate_keys("{\"a\": 1} extra")
            .unwrap()
            .is_err());
    }
}
