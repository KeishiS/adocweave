//! Unknown-field rejection shared by every public request object.

use std::collections::BTreeMap;

use serde::Deserialize;

// `serde_wasm_bindgen` 0.6 deserializes a typed struct by requesting known
// properties and does not enumerate extra JavaScript properties for
// `deny_unknown_fields`. Flattening this rejecting map forces map traversal at
// the WASM boundary. `protocol-wasm.test.mjs` exercises every nested request
// object with the generated module; JSON deserialization rejects the same
// fields.
pub(crate) type UnknownFields = BTreeMap<String, UnknownField>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UnknownField;

impl<'de> Deserialize<'de> for UnknownField {
    fn deserialize<D>(_: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Err(serde::de::Error::custom("unknown field"))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    const REQUEST_STRUCTS: &[&str] = &[
        "ActiveUrlOptions",
        "AnalyzeRequest",
        "AuthoredUrlOptions",
        "CitationSegment",
        "DiagnosticOptions",
        "ExternalLinkOptions",
        "GeneratedBibliography",
        "GeneratedBibliographyEntry",
        "HtmlOptions",
        "ProductRequest",
        "ResolvedCitation",
        "ResolvedReference",
        "ResolvedResource",
        "ResourceCapabilities",
        "ResourceInput",
        "RoleOptions",
        "RuleOptions",
        "SourceInput",
        "SourceLanguageOptions",
    ];
    const REQUEST_OBJECT_ENUMS: &[&str] = &[
        "CitationOutcome",
        "ReferenceOutcome",
        "ResourceOutcome",
        "Stylesheet",
    ];

    #[test]
    fn every_public_request_struct_has_the_common_unknown_field_guard() {
        let sources = [
            include_str!("request_wire.rs"),
            include_str!("render_input_wire.rs"),
        ];
        let mut actual = BTreeSet::new();
        for source in sources {
            let declarations = source
                .match_indices("pub struct ")
                .map(|(offset, _)| offset)
                .collect::<Vec<_>>();
            for (index, start) in declarations.iter().copied().enumerate() {
                let declaration = &source[start + "pub struct ".len()..];
                let name = declaration
                    .split(|character: char| !character.is_ascii_alphanumeric())
                    .next()
                    .expect("public struct name");
                let end = declarations.get(index + 1).copied().unwrap_or(source.len());
                let body = &source[start..end];
                assert!(
                    body.contains("unknown_fields: UnknownFields"),
                    "public request struct {name} lacks the common unknown-field guard"
                );
                actual.insert(name);
            }
        }
        assert_eq!(
            actual,
            REQUEST_STRUCTS.iter().copied().collect(),
            "update the public request object inventory when adding an input struct"
        );
    }

    #[test]
    fn every_object_enum_uses_a_guarded_deserialization_type() {
        let sources = [
            include_str!("request_wire.rs"),
            include_str!("render_input_wire.rs"),
        ];
        let mut actual = BTreeSet::new();
        for source in sources {
            for (start, _) in source.match_indices("pub enum ") {
                let tail = &source[start + "pub enum ".len()..];
                let name = tail
                    .split(|character: char| !character.is_ascii_alphanumeric())
                    .next()
                    .expect("public enum name");
                let open = tail.find('{').expect("enum body");
                let mut depth = 0_u32;
                let mut close = open;
                for (offset, character) in tail[open..].char_indices() {
                    match character {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                close = open + offset;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                if tail[open + 1..close].contains('{') {
                    assert!(
                        source.contains(&format!("enum {name}Input")),
                        "request object enum {name} lacks a guarded deserialization type"
                    );
                    actual.insert(name);
                }
            }
        }
        assert_eq!(
            actual,
            REQUEST_OBJECT_ENUMS.iter().copied().collect(),
            "update the request object enum inventory when adding an object variant"
        );
    }
}
