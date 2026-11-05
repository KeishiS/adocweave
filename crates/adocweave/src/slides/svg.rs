//! A finite SVG profile for local research figures, separate from MathJax output.

use std::collections::BTreeSet;

const MAX_BYTES: usize = 10 * 1024 * 1024;
const MAX_NODES: u32 = 16_384;
const MAX_DEPTH: usize = 64;
const SVG: &str = "http://www.w3.org/2000/svg";
const XLINK: &str = "http://www.w3.org/1999/xlink";

fn id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte))
}

fn reference(value: &str, references: &mut BTreeSet<String>) -> bool {
    value
        .strip_prefix('#')
        .filter(|value| id(value))
        .is_some_and(|value| {
            references.insert(value.to_owned());
            true
        })
}

fn css_value(value: &str, references: &mut BTreeSet<String>) -> bool {
    if value.is_empty()
        || !value.is_ascii()
        || value.contains(['\\', '@', '{', '}', ';', ':', '<', '>', '"', '\''])
        || value.contains("/*")
    {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    if lower.contains("url(") {
        return lower
            .strip_prefix("url(")
            .and_then(|value| value.strip_suffix(')'))
            .is_some_and(|_| {
                value
                    .strip_prefix("url(")
                    .and_then(|value| value.strip_suffix(')'))
                    .is_some_and(|value| reference(value.trim(), references))
            });
    }
    // No escaping, variables, at-rules or asset-bearing functions.
    if value.contains('(') {
        return ["rgb(", "rgba(", "hsl(", "hsla("]
            .iter()
            .find_map(|prefix| lower.strip_prefix(prefix))
            .and_then(|body| body.strip_suffix(')'))
            .is_some_and(|body| {
                body.bytes().all(|byte| {
                    byte.is_ascii_digit()
                        || b".,%+- /".contains(&byte)
                        || byte.is_ascii_whitespace()
                })
            });
    }
    value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || b"#.,%+-() _/".contains(&byte) || byte.is_ascii_whitespace()
    })
}

fn font_value(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_alphanumeric()
                || "_.,%+- '\"".contains(character)
                || character.is_whitespace()
        })
}

fn declarations(value: &str, references: &mut BTreeSet<String>) -> bool {
    value
        .split(';')
        .filter(|value| !value.trim().is_empty())
        .all(|declaration| {
            let Some((name, value)) = declaration.split_once(':') else {
                return false;
            };
            let name = name.trim();
            if matches!(name, "font" | "font-family") {
                return font_value(value.trim());
            }
            matches!(
                name,
                "fill"
                    | "fill-opacity"
                    | "fill-rule"
                    | "stroke"
                    | "stroke-width"
                    | "stroke-opacity"
                    | "stroke-linecap"
                    | "stroke-linejoin"
                    | "stroke-miterlimit"
                    | "stroke-dasharray"
                    | "stroke-dashoffset"
                    | "opacity"
                    | "color"
                    | "font-family"
                    | "font-size"
                    | "font-style"
                    | "font-weight"
                    | "font-stretch"
                    | "text-anchor"
                    | "dominant-baseline"
                    | "alignment-baseline"
                    | "letter-spacing"
                    | "word-spacing"
                    | "white-space"
                    | "display"
                    | "visibility"
                    | "clip-path"
                    | "marker-start"
                    | "marker-mid"
                    | "marker-end"
                    | "stop-color"
                    | "stop-opacity"
                    | "vector-effect"
                    | "paint-order"
            ) && css_value(value.trim(), references)
        })
}

fn stylesheet(value: &str, references: &mut BTreeSet<String>) -> bool {
    if value.contains(['\\', '@']) || value.contains("/*") {
        return false;
    }
    let mut remaining = value.trim();
    while !remaining.is_empty() {
        let Some((selector, rest)) = remaining.split_once('{') else {
            return false;
        };
        if !selector.split(',').all(|selector| {
            let selector = selector.trim();
            selector == "*"
                || selector
                    .strip_prefix(['.', '#'])
                    .map_or_else(|| id(selector), id)
        }) {
            return false;
        }
        let Some((body, rest)) = rest.split_once('}') else {
            return false;
        };
        if !declarations(body, references) {
            return false;
        }
        remaining = rest.trim();
    }
    true
}

fn escape(value: &str, attribute: bool) -> String {
    let value = value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    if attribute {
        value.replace('"', "&quot;")
    } else {
        value
    }
}

/// Validates and serializes owned XML; authored markup is never copied verbatim.
pub(super) fn validate(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    if bytes.len() > MAX_BYTES {
        return Err("SVG exceeds the byte limit");
    }
    let source = std::str::from_utf8(bytes).map_err(|_| "SVG must be UTF-8")?;
    if source.contains("<!DOCTYPE") || source.contains("<!ENTITY") {
        return Err("SVG document types and entities are not supported");
    }
    let document = roxmltree::Document::parse_with_options(
        source,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: MAX_NODES,
            ..Default::default()
        },
    )
    .map_err(|_| "SVG must be well-formed XML without a document type")?;
    let root = document.root_element();
    if root.tag_name().name() != "svg" || root.tag_name().namespace() != Some(SVG) {
        return Err("SVG root must use the SVG namespace");
    }
    let mut ids = BTreeSet::new();
    let mut references = BTreeSet::new();
    for node in root.descendants() {
        if node.ancestors().count() > MAX_DEPTH {
            return Err("SVG exceeds the depth limit");
        }
        if node.is_pi() {
            return Err("SVG processing instructions are not supported");
        }
        if !node.is_element() {
            continue;
        }
        if node.tag_name().namespace() != Some(SVG)
            || !matches!(
                node.tag_name().name(),
                "svg"
                    | "g"
                    | "path"
                    | "rect"
                    | "circle"
                    | "ellipse"
                    | "line"
                    | "polyline"
                    | "polygon"
                    | "text"
                    | "tspan"
                    | "textPath"
                    | "title"
                    | "desc"
                    | "defs"
                    | "symbol"
                    | "use"
                    | "clipPath"
                    | "marker"
                    | "linearGradient"
                    | "radialGradient"
                    | "stop"
                    | "style"
            )
        {
            return Err("SVG contains an unsupported element");
        }
        for attribute in node.attributes() {
            let name = attribute.name();
            let value = attribute.value();
            if !matches!(
                attribute.namespace(),
                None | Some(XLINK) | Some("http://www.w3.org/XML/1998/namespace")
            ) {
                return Err("SVG contains an unsupported attribute namespace");
            }
            let valid = match name {
                "id" => id(value) && ids.insert(value.to_owned()),
                "href" => reference(value, &mut references),
                "style" => declarations(value, &mut references),
                "class" => value.split_ascii_whitespace().all(id),
                "fill" | "stroke" | "clip-path" | "marker-start" | "marker-mid" | "marker-end"
                | "stop-color" | "color" => css_value(value, &mut references),
                "font-family" => font_value(value),
                "d" | "points" | "transform" | "gradientTransform" | "viewBox" | "x" | "y"
                | "x1" | "x2" | "y1" | "y2" | "cx" | "cy" | "r" | "rx" | "ry" | "fx" | "fy"
                | "width" | "height" | "dx" | "dy" | "rotate" | "offset" | "stroke-width"
                | "stroke-dasharray" | "stroke-dashoffset" | "stroke-miterlimit" | "opacity"
                | "fill-opacity" | "stroke-opacity" | "stop-opacity" | "font-size"
                | "letter-spacing" | "word-spacing" | "markerWidth" | "markerHeight" | "refX"
                | "refY" | "orient" | "startOffset" => {
                    value.is_ascii()
                        && value.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric()
                                || b".,%+-() ".contains(&byte)
                                || byte.is_ascii_whitespace()
                        })
                }
                "version"
                | "preserveAspectRatio"
                | "fill-rule"
                | "clip-rule"
                | "stroke-linecap"
                | "stroke-linejoin"
                | "gradientUnits"
                | "spreadMethod"
                | "clipPathUnits"
                | "markerUnits"
                | "font-style"
                | "font-weight"
                | "font-stretch"
                | "text-anchor"
                | "dominant-baseline"
                | "alignment-baseline"
                | "display"
                | "visibility"
                | "vector-effect"
                | "paint-order"
                | "space"
                | "type" => {
                    value.is_ascii()
                        && value
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || b"-./ ".contains(&byte))
                }
                _ => false,
            };
            if !valid {
                return Err("SVG contains an unsupported or unsafe attribute");
            }
        }
        if node.tag_name().name() == "style" {
            if node.children().any(|child| child.is_element()) {
                return Err("SVG style must contain only CSS text");
            }
            let css = node
                .children()
                .filter_map(|child| child.text())
                .collect::<String>();
            if !stylesheet(&css, &mut references) {
                return Err("SVG contains unsupported CSS");
            }
        }
    }
    if !references.is_subset(&ids) {
        return Err("SVG references an unknown local ID");
    }
    fn write(node: roxmltree::Node<'_, '_>, output: &mut String) {
        if node.is_text() {
            output.push_str(&escape(node.text().unwrap_or(""), false));
            return;
        }
        if !node.is_element() {
            return;
        }
        output.push('<');
        output.push_str(node.tag_name().name());
        if node.parent().is_some_and(|parent| parent.is_root()) {
            output.push_str(" xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"");
        }
        for attribute in node.attributes() {
            output.push(' ');
            if attribute.namespace() == Some(XLINK) {
                output.push_str("xlink:");
            } else if attribute.namespace() == Some("http://www.w3.org/XML/1998/namespace") {
                output.push_str("xml:");
            }
            output.push_str(attribute.name());
            output.push_str("=\"");
            output.push_str(&escape(attribute.value(), true));
            output.push('"');
        }
        output.push('>');
        for child in node.children() {
            write(child, output);
        }
        output.push_str("</");
        output.push_str(node.tag_name().name());
        output.push('>');
    }
    let mut output = String::new();
    write(root, &mut output);
    if output.len() > MAX_BYTES {
        return Err("serialized SVG exceeds the byte limit");
    }
    Ok(output.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn research_shapes_text_gradients_and_local_references_are_preserved() {
        let svg = r##"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 300 200"><defs><style>*{stroke-linejoin:round;stroke-linecap:butt}</style><linearGradient id="color"><stop offset="0" stop-color="#abc"/></linearGradient><clipPath id="clip"><rect width="100" height="100"/></clipPath><path id="shape" d="M 0 0 L 100 100"/></defs><g style="fill:url(#color);stroke:#222" clip-path="url(#clip)"><use xlink:href="#shape"/><text x="20" y="50" style="font: 12px 'DejaVu Sans';fill:rgb(20,30,40)">実験 &amp; 結果<tspan>2</tspan></text></g></svg>"##;
        let output = String::from_utf8(validate(svg.as_bytes()).unwrap()).unwrap();
        assert!(output.contains("実験 &amp; 結果"));
        assert!(output.contains("xlink:href=\"#shape\""));
        assert!(!output.contains("<?xml"));
    }
    #[test]
    fn executable_external_and_escaped_css_are_rejected() {
        for content in [
            "<script>alert(1)</script>",
            "<foreignObject/>",
            "<rect onclick=\"alert(1)\"/>",
            "<use href=\"https://example.com/x.svg#x\"/>",
            "<rect style=\"fill:url(https://example.com/x)\"/>",
            "<rect style=\"fill:u\\72l(#x)\"/>",
            "<style>@import 'https://example.com/x';</style>",
            "<use href=\"#missing\"/>",
            "<style><text>@import 'https://example.com/x';</text></style>",
            "<rect fill=\"rgb(var(--color))\"/>",
        ] {
            assert!(
                validate(format!("<svg xmlns=\"{SVG}\">{content}</svg>").as_bytes()).is_err(),
                "{content}"
            );
        }
        assert!(validate(format!("<!DOCTYPE svg><svg xmlns=\"{SVG}\"/>").as_bytes()).is_err());
        assert!(
            validate(
                format!(
                    "<svg xmlns=\"{SVG}\">{}{}{}</svg>",
                    "<g>".repeat(65),
                    "text",
                    "</g>".repeat(65)
                )
                .as_bytes()
            )
            .is_err()
        );
    }
}
