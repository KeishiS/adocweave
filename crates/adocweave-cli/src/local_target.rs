//! Explicit filesystem validation for typed local targets.

use adocweave::text::{PositionEncoding, SourceDocument};
#[derive(Clone, Debug)]
pub struct HostDiagnostic {
    pub code: String,
    pub message: String,
    pub source_id: String,
    pub range: adocweave::text::TextRange,
    pub target: String,
    pub line: u32,
    pub column: u32,
}

pub(crate) fn diagnostic_from_project(
    diagnostic: &adocweave::output::diagnostics::Diagnostic,
    source_id: &str,
    source: &str,
    target: &str,
) -> HostDiagnostic {
    let (line, column) = line_column(source, diagnostic.range.start().to_u32() as usize);
    HostDiagnostic {
        code: diagnostic.code.as_str().to_owned(),
        message: diagnostic.message.clone(),
        source_id: source_id.to_owned(),
        range: diagnostic.range,
        target: target.to_owned(),
        line,
        column,
    }
}

pub fn render_human(
    diagnostics: &[HostDiagnostic],
    source: &str,
) -> Result<String, adocweave::text::PositionError> {
    use std::fmt::Write as _;

    let document = SourceDocument::new(source)?;
    let mut output = String::new();
    for diagnostic in diagnostics {
        let _ = document.offset_to_position(diagnostic.range.start(), PositionEncoding::Utf8)?;
        let source_id = visible(&diagnostic.source_id);
        let target = visible(&diagnostic.target);
        writeln!(
            output,
            "{}:{}:{}: error[{}]: {} (target: {})",
            source_id,
            diagnostic.line,
            diagnostic.column,
            diagnostic.code,
            diagnostic.message,
            target
        )
        .expect("writing to a String cannot fail");
    }
    Ok(output)
}

fn visible(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if character.is_control() {
            output.extend(character.escape_default());
        } else {
            output.push(character);
        }
    }
    output
}

pub fn json_values(diagnostics: &[HostDiagnostic]) -> Vec<serde_json::Value> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            serde_json::json!({
                "id": format!(
                    "{}@{}:{}:{}",
                    diagnostic.code,
                    diagnostic.source_id,
                    diagnostic.range.start().to_u32(),
                    diagnostic.range.end().to_u32()
                ),
                "code": diagnostic.code,
                "severity": "error",
                "message": diagnostic.message,
                "sourceId": diagnostic.source_id,
                "range": {
                    "start": diagnostic.range.start().to_u32(),
                    "end": diagnostic.range.end().to_u32()
                },
                "target": diagnostic.target,
                "line": diagnostic.line,
                "column": diagnostic.column
            })
        })
        .map(|value| match value {
            // Local target diagnostics carry no related information and no fix,
            // but every record emits the same keys.
            serde_json::Value::Object(object) => crate::diagnostic_json::with_common_keys(object),
            other => other,
        })
        .collect()
}

fn line_column(source: &str, offset: usize) -> (u32, u32) {
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u32 + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, tail)| tail)
        .len() as u32
        + 1;
    (line, column)
}
