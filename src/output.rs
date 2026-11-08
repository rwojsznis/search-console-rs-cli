//! What goes on stdout (decisions 3 and 4), and the warning lines on stderr.
//! The bytes do not depend on whether stdout is a terminal.

use crate::{errln, out};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    /// Pretty JSON, 2-space indent.
    Json,
    /// One compact JSON value per line: one row, list item or request each.
    Jsonl,
    /// A header line, then one line per row. `query` and `compare` only.
    Csv,
}

/// The text for one value, with its trailing newline.
#[must_use]
pub fn render(value: &Value, format: Format) -> String {
    match format {
        Format::Json => format!("{value:#}\n"),
        Format::Jsonl | Format::Csv => format!("{value}\n"),
    }
}

/// Prints one value. Returns `false` when stdout is closed.
#[expect(
    clippy::must_use_candidate,
    reason = "a command that prints one value has nothing to do when stdout is closed; only the paging loops read the result"
)]
pub fn print(value: &Value, format: Format) -> bool {
    out!("{}", render(value, format))
}

/// Prints one response. For `jsonl` and a list endpoint, that is one line
/// per item of `list_key`. Google leaves the key out when the list is empty,
/// so a body without it prints nothing in `jsonl`.
pub fn print_response(value: &Value, list_key: Option<&str>, format: Format) {
    let (Format::Jsonl, Some(key)) = (format, list_key) else {
        print(value, format);
        return;
    };
    if let Some(items) = value.get(key).and_then(Value::as_array) {
        // `all` stops at the first write that finds stdout closed.
        let _ = items.iter().all(|item| print(item, Format::Jsonl));
    }
}

/// One CSV line (RFC 4180), with its trailing newline. A string is quoted
/// when it holds a comma, a quote or a line break; `null` is an empty cell.
#[must_use]
pub fn csv_line<'a>(cells: impl IntoIterator<Item = &'a Value>) -> String {
    let cells: Vec<String> = cells.into_iter().map(csv_cell).collect();
    let mut line = cells.join(",");
    line.push('\n');
    line
}

fn csv_cell(value: &Value) -> String {
    let text = match value {
        Value::Null => return String::new(),
        Value::String(text) => text.clone(),
        Value::Bool(_) | Value::Number(_) => return value.to_string(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    };
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

/// One warning line on stderr. Warnings do not change the exit code
/// (decision 5).
pub fn warn(kind: &str, message: &str) {
    errln!("{}", json!({"warning": {"kind": kind, "message": message}}));
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = r#"{"b":1,"a":{"z":[1,2],"y":"ü"}}"#;

    #[test]
    fn json_is_pretty_with_two_spaces_and_keeps_the_key_order() {
        let value: Value = serde_json::from_str(BODY).unwrap();
        assert_eq!(
            render(&value, Format::Json),
            "{\n  \"b\": 1,\n  \"a\": {\n    \"z\": [\n      1,\n      2\n    ],\n    \"y\": \"ü\"\n  }\n}\n"
        );
    }

    #[test]
    fn jsonl_is_the_compact_body_on_one_line() {
        let value: Value = serde_json::from_str(BODY).unwrap();
        assert_eq!(render(&value, Format::Jsonl), format!("{BODY}\n"));
    }

    #[test]
    fn csv_quotes_only_what_needs_it() {
        let cells = [
            json!("plain"),
            json!("a,b"),
            json!("say \"hi\""),
            json!("two\nlines"),
            json!(12),
            json!(0.25),
            Value::Null,
            json!(true),
            json!(["x"]),
        ];
        assert_eq!(
            csv_line(&cells),
            "plain,\"a,b\",\"say \"\"hi\"\"\",\"two\nlines\",12,0.25,,true,\"[\"\"x\"\"]\"\n"
        );
    }
}
