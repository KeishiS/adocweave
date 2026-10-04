//! Finite, normalized host-rendered math and citation content.

use std::collections::BTreeSet;
use std::fmt;

use crate::source::TextRange;
use crate::url::{ActiveUrlPolicy, UrlProvenance};

const XML_BYTES: usize = 512 * 1024;
const XML_NODES: u32 = 16_384;
const XML_DEPTH: usize = 64;
const RICH_NODES: usize = 4096;
const RICH_BYTES: usize = 256 * 1024;
const RICH_DEPTH: usize = 32;

/// Rejection of content outside the finite renderer profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentValidationError(String);
impl fmt::Display for ContentValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ContentValidationError {}
fn invalid(message: impl Into<String>) -> ContentValidationError {
    ContentValidationError(message.into())
}

/// Normalized SVG and assistive MathML. Markup is never caller-writable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedMath {
    svg: String,
    mathml: String,
    ids: BTreeSet<String>,
    references: BTreeSet<String>,
}
impl ValidatedMath {
    /// Validates the MathJax SVG/MathML profile with fixed byte, node and depth limits.
    /// IDs belong to this equation; links may address another equation in the same scope.
    pub fn validate(
        scope: &str,
        key: &str,
        svg: &str,
        mathml: &str,
    ) -> Result<Self, ContentValidationError> {
        if !matches!(scope, "body" | "notes") || !is_key(key) {
            return Err(invalid("invalid math scope or key"));
        }
        let mut ids = BTreeSet::new();
        let mut references = BTreeSet::new();
        let svg = normalize_xml(svg, "svg", scope, key, &mut ids, &mut references)?;
        let mathml = normalize_xml(mathml, "math", scope, key, &mut ids, &mut references)?;
        Ok(Self {
            svg,
            mathml,
            ids,
            references,
        })
    }
    /// SVG IDs used by the host to reject collisions across output regions.
    pub fn ids(&self) -> &BTreeSet<String> {
        &self.ids
    }
    /// Fragment targets whose existence the host must check across this scope.
    pub fn references(&self) -> &BTreeSet<String> {
        &self.references
    }
    pub(crate) fn svg(&self) -> &str {
        &self.svg
    }
    pub(crate) fn mathml(&self) -> &str {
        &self.mathml
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedMath {
    pub source_range: TextRange,
    pub(crate) value: ValidatedMath,
}
impl ResolvedMath {
    pub const fn new(source_range: TextRange, value: ValidatedMath) -> Self {
        Self {
            source_range,
            value,
        }
    }
    pub fn value(&self) -> &ValidatedMath {
        &self.value
    }
}

/// The only formatting operations accepted from a citation processor.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum RichInline {
    Text { text: String },
    Emphasis { children: Vec<Self> },
    Strong { children: Vec<Self> },
    Superscript { children: Vec<Self> },
    Subscript { children: Vec<Self> },
    Smallcaps { children: Vec<Self> },
    NormalEmphasis { children: Vec<Self> },
    NormalStrong { children: Vec<Self> },
    NormalSmallcaps { children: Vec<Self> },
    Underline { children: Vec<Self> },
    Link { href: String, children: Vec<Self> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedRichText {
    html: String,
    text: String,
    links: Vec<String>,
}
impl ValidatedRichText {
    pub fn validate(inlines: Vec<RichInline>) -> Result<Self, ContentValidationError> {
        let mut result = Self {
            html: String::new(),
            text: String::new(),
            links: Vec::new(),
        };
        let mut count = 0;
        result.append(&inlines, 0, &mut count)?;
        Ok(result)
    }
    pub fn plain_text(&self) -> &str {
        &self.text
    }
    /// Active URLs retained for the host's URL parser and policy checks.
    pub fn links(&self) -> &[String] {
        &self.links
    }
    pub(crate) fn html(&self) -> &str {
        &self.html
    }
    pub(crate) fn allowed_by(&self, policy: &ActiveUrlPolicy) -> bool {
        self.links
            .iter()
            .all(|href| policy.allows(href, UrlProvenance::ResolvedReference))
    }
    fn check_size(&self) -> Result<(), ContentValidationError> {
        if self.html.len() > RICH_BYTES * 6 {
            Err(invalid("citation output byte limit exceeded"))
        } else {
            Ok(())
        }
    }
    fn append(
        &mut self,
        nodes: &[RichInline],
        depth: usize,
        count: &mut usize,
    ) -> Result<(), ContentValidationError> {
        if depth > RICH_DEPTH {
            return Err(invalid("citation inline depth limit exceeded"));
        }
        for node in nodes {
            *count += 1;
            if *count > RICH_NODES {
                return Err(invalid("citation inline count limit exceeded"));
            }
            let (tag, class, children) = match node {
                RichInline::Text { text } => {
                    if text.len() > RICH_BYTES.saturating_sub(self.text.len()) {
                        return Err(invalid("citation text byte limit exceeded"));
                    }
                    self.text.push_str(text);
                    escape(&mut self.html, text);
                    self.check_size()?;
                    continue;
                }
                RichInline::Emphasis { children } => ("em", None, children),
                RichInline::Strong { children } => ("strong", None, children),
                RichInline::Superscript { children } => ("sup", None, children),
                RichInline::Subscript { children } => ("sub", None, children),
                RichInline::Smallcaps { children } => ("span", Some("csl-smallcaps"), children),
                RichInline::NormalEmphasis { children } => {
                    ("span", Some("csl-normal-emphasis"), children)
                }
                RichInline::NormalStrong { children } => {
                    ("span", Some("csl-normal-strong"), children)
                }
                RichInline::NormalSmallcaps { children } => {
                    ("span", Some("csl-normal-smallcaps"), children)
                }
                RichInline::Underline { children } => ("u", None, children),
                RichInline::Link { href, children } => {
                    if href.len() > 4096 || !citation_url(href) {
                        return Err(invalid(
                            "citation link must be an absolute HTTP(S) URL without credentials",
                        ));
                    }
                    self.links.push(href.clone());
                    self.html.push_str("<a href=\"");
                    escape(&mut self.html, href);
                    self.html.push_str("\">");
                    self.append(children, depth + 1, count)?;
                    self.html.push_str("</a>");
                    self.check_size()?;
                    continue;
                }
            };
            self.html.push('<');
            self.html.push_str(tag);
            if let Some(class) = class {
                self.html.push_str(" class=\"");
                self.html.push_str(class);
                self.html.push('"');
            }
            self.html.push('>');
            self.append(children, depth + 1, count)?;
            self.html.push_str("</");
            self.html.push_str(tag);
            self.html.push('>');
            if self.html.len() > RICH_BYTES * 6 {
                return Err(invalid("citation output byte limit exceeded"));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedRichCitation {
    pub source_range: TextRange,
    pub(crate) value: ValidatedRichText,
}
impl ResolvedRichCitation {
    pub const fn new(source_range: TextRange, value: ValidatedRichText) -> Self {
        Self {
            source_range,
            value,
        }
    }
    pub fn value(&self) -> &ValidatedRichText {
        &self.value
    }
}

fn citation_url(value: &str) -> bool {
    if !ActiveUrlPolicy::default().allows(value, UrlProvenance::ResolvedReference)
        || value.contains('\\')
    {
        return false;
    }
    let Some((scheme, rest)) = value.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    !authority.is_empty() && !authority.contains('@')
}

fn is_key(value: &str) -> bool {
    value.len() <= 64
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
fn is_id(value: &str, scope: &str, key: Option<&str>) -> bool {
    let Some(rest) = value.strip_prefix(&format!("{scope}-")) else {
        return false;
    };
    let Some((id_key, number)) = rest.rsplit_once("-i") else {
        return false;
    };
    value.len() <= 128
        && is_key(id_key)
        && number.len() <= 10
        && number.parse::<u32>().is_ok()
        && key.is_none_or(|key| key == id_key)
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
}

fn normalize_xml(
    source: &str,
    root: &str,
    scope: &str,
    key: &str,
    ids: &mut BTreeSet<String>,
    refs: &mut BTreeSet<String>,
) -> Result<String, ContentValidationError> {
    if source.len() > XML_BYTES {
        return Err(invalid("math XML byte limit exceeded"));
    }
    // Reject declarations, DTDs (including an empty DTD), comments, CDATA and PIs.
    // Standard escaped/numeric characters are decoded by the XML parser and re-escaped below.
    if source.contains("<!") || source.contains("<?") {
        return Err(invalid("math XML declarations and DTDs are forbidden"));
    }
    let document = roxmltree::Document::parse_with_options(
        source,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: XML_NODES,
            ..Default::default()
        },
    )
    .map_err(|error| invalid(format!("invalid math XML: {error}")))?;
    let element = document.root_element();
    if element.tag_name().name() != root {
        return Err(invalid("unexpected math XML root"));
    }
    let namespace = if root == "svg" {
        "http://www.w3.org/2000/svg"
    } else {
        "http://www.w3.org/1998/Math/MathML"
    };
    let mut output = String::new();
    XmlNormalizer {
        namespace,
        scope,
        key,
        ids,
        references: refs,
    }
    .node(element, 0, &mut output)?;
    if output.len() > XML_BYTES * 6 {
        return Err(invalid("normalized math XML byte limit exceeded"));
    }
    Ok(output)
}
struct XmlNormalizer<'a> {
    namespace: &'a str,
    scope: &'a str,
    key: &'a str,
    ids: &'a mut BTreeSet<String>,
    references: &'a mut BTreeSet<String>,
}
impl XmlNormalizer<'_> {
    fn node(
        &mut self,
        node: roxmltree::Node<'_, '_>,
        depth: usize,
        output: &mut String,
    ) -> Result<(), ContentValidationError> {
        let namespace = self.namespace;
        let scope = self.scope;
        let key = self.key;
        if depth > XML_DEPTH {
            return Err(invalid("math XML depth limit exceeded"));
        }
        if node.is_text() {
            escape(output, node.text().unwrap_or_default());
            return Ok(());
        }
        if !node.is_element() {
            return Err(invalid("unsupported math XML node"));
        }
        let svg = namespace == "http://www.w3.org/2000/svg";
        let tag = node.tag_name();
        if tag.namespace() != Some(namespace) || !allowed_element(svg, tag.name()) {
            return Err(invalid("unsupported math XML element or namespace"));
        }
        if node
            .namespaces()
            .any(|ns| ns.name().is_some() || ns.uri() != namespace)
        {
            return Err(invalid("unsupported math XML namespace declaration"));
        }
        output.push('<');
        output.push_str(tag.name());
        if depth == 0 {
            output.push_str(" xmlns=\"");
            output.push_str(namespace);
            output.push('"');
        }
        for attr in node.attributes() {
            let name = attr.name();
            let value = attr.value();
            if attr.namespace().is_some() {
                return Err(invalid("namespaced math XML attributes are forbidden"));
            }
            match name {
                "id" if svg && is_id(value, scope, Some(key)) => {
                    if !self.ids.insert(value.to_owned()) {
                        return Err(invalid("duplicate math SVG ID"));
                    }
                }
                "href" => {
                    let target = value
                        .strip_prefix('#')
                        .ok_or_else(|| invalid("math links must be local fragments"))?;
                    if !is_id(target, scope, (tag.name() == "use").then_some(key)) {
                        return Err(invalid("math link is outside its scope"));
                    }
                    if svg && !matches!(tag.name(), "a" | "use") {
                        return Err(invalid("unexpected SVG link"));
                    }
                    self.references.insert(target.to_owned());
                }
                _ if allowed_attribute(svg, name, value) => {}
                _ => return Err(invalid(format!("unsupported math XML attribute `{name}`"))),
            }
            output.push(' ');
            output.push_str(name);
            output.push_str("=\"");
            escape(output, value);
            output.push('"');
        }
        output.push('>');
        for child in node.children() {
            self.node(child, depth + 1, output)?;
        }
        output.push_str("</");
        output.push_str(tag.name());
        output.push('>');
        Ok(())
    }
}

fn allowed_element(svg: bool, name: &str) -> bool {
    if svg {
        matches!(
            name,
            "svg"
                | "defs"
                | "path"
                | "g"
                | "a"
                | "rect"
                | "use"
                | "text"
                | "line"
                | "polygon"
                | "polyline"
        )
    } else {
        matches!(
            name,
            "math"
                | "mrow"
                | "mi"
                | "mn"
                | "mo"
                | "mtext"
                | "mspace"
                | "ms"
                | "mfrac"
                | "msqrt"
                | "mroot"
                | "mstyle"
                | "merror"
                | "mpadded"
                | "mphantom"
                | "mfenced"
                | "menclose"
                | "msub"
                | "msup"
                | "msubsup"
                | "munder"
                | "mover"
                | "munderover"
                | "mmultiscripts"
                | "mprescripts"
                | "none"
                | "mtable"
                | "mtr"
                | "mlabeledtr"
                | "mtd"
        )
    }
}
fn numeric(value: &str) -> bool {
    !value.is_empty() && value.parse::<f64>().is_ok_and(f64::is_finite)
}
fn numbers(value: &str) -> bool {
    !value.trim().is_empty()
        && value
            .split(|c: char| c.is_ascii_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .all(numeric)
}
fn length(value: &str) -> bool {
    numeric(value)
        || ["ex", "em", "px", "pt", "%"]
            .iter()
            .any(|suffix| value.strip_suffix(suffix).is_some_and(numeric))
}
fn tokens(value: &str) -> bool {
    value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b' ' | b'_' | b'-' | b'.'))
}
fn transform(value: &str) -> bool {
    let mut remaining = value.trim();
    while !remaining.is_empty() {
        let Some((name, rest)) = remaining.split_once('(') else {
            return false;
        };
        if !matches!(
            name.trim(),
            "translate" | "scale" | "matrix" | "rotate" | "skewX" | "skewY"
        ) {
            return false;
        }
        let Some((args, rest)) = rest.split_once(')') else {
            return false;
        };
        if !numbers(args) {
            return false;
        }
        remaining = rest.trim();
    }
    true
}
fn style(value: &str) -> bool {
    value
        .split(';')
        .filter(|p| !p.trim().is_empty())
        .all(|part| {
            let Some((name, value)) = part.split_once(':') else {
                return false;
            };
            match name.trim() {
                "vertical-align" | "min-width" | "min-height" => length(value.trim()),
                "overflow" => value.trim() == "visible",
                _ => false,
            }
        })
}
fn allowed_attribute(svg: bool, name: &str, value: &str) -> bool {
    if svg {
        match name {
            "style" => style(value),
            "width" | "height" | "x" | "y" | "x1" | "x2" | "y1" | "y2" | "rx" | "ry"
            | "font-size" => length(value),
            "viewBox" | "data-mjx-viewBox" | "points" | "stroke-dasharray" => numbers(value),
            "transform" => transform(value),
            "d" => value.bytes().all(|b| {
                b.is_ascii_digit()
                    || b.is_ascii_whitespace()
                    || b"MmZzLlHhVvCcSsQqTtAaEe.,+-".contains(&b)
            }),
            "stroke" | "fill" => matches!(value, "currentColor" | "none"),
            "stroke-width" | "opacity" => numeric(value),
            "stroke-linecap" => matches!(value, "round" | "butt" | "square"),
            "role" => value == "img",
            "focusable" | "aria-hidden" => matches!(value, "true" | "false"),
            "preserveAspectRatio" => matches!(value, "xMidYMid" | "xMaxYMid" | "xMinYMid" | "none"),
            "pointer-events" => value == "all",
            "font-family" => matches!(value, "serif" | "sans-serif" | "monospace"),
            "class" => value
                .split_ascii_whitespace()
                .all(|s| matches!(s, "MathJax_ref" | "mjx-dashed" | "mjx-dotted" | "mjx-solid")),
            "data-hitbox" | "data-id-align" | "data-idbox" | "data-table" | "data-labels"
            | "data-line" | "data-frame" => matches!(value, "true" | "h" | "v"),
            "data-c" => {
                !value.is_empty() && value.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
            }
            "data-mml-node" => {
                allowed_element(false, value) || matches!(value, "TeXAtom" | "inferredMrow")
            }
            "data-variant" | "data-mjx-texclass" | "data-frame-styles" => tokens(value),
            "data-padding" => numeric(value),
            _ => false,
        }
    } else {
        match name {
            "display" => matches!(value, "block" | "inline"),
            "class" => value == "MathJax_ref",
            "mathvariant" | "form" | "columnalign" | "rowalign" | "groupalign" | "align"
            | "notation" | "columnlines" | "rowlines" | "frame" | "data-break-align"
            | "data-mjx-texclass" | "data-frame-styles" => tokens(value),
            "displaystyle" | "stretchy" | "symmetric" | "largeop" | "movablelimits" | "accent"
            | "accentunder" | "fence" | "separator" | "equalrows" | "equalcolumns" | "bevelled" => {
                matches!(value, "true" | "false")
            }
            "scriptlevel" | "rowspan" | "columnspan" | "data-padding" => numeric(value),
            "width" | "height" | "depth" | "lspace" | "rspace" | "voffset" | "linethickness"
            | "minsize" | "maxsize" | "indentshift" | "mathsize" => {
                length(value) || matches!(value, "infinity" | "thin" | "medium" | "thick")
            }
            "columnspacing" | "rowspacing" | "framespacing" => {
                value.split_ascii_whitespace().all(length)
            }
            "open" | "close" | "separators" => {
                value.len() <= 64 && !value.chars().any(char::is_control)
            }
            _ => false,
        }
    }
}
pub(crate) fn escape(output: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            _ => output.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="1ex" height="2ex" viewBox="0 0 1 2" aria-hidden="true"><defs><path id="body-m0-i0" d="M0 0L1 2Z"/></defs><g transform="scale(1,-1)"><use href="#body-m0-i0"/></g></svg>"##;
    const MATHML: &str = r#"<math xmlns="http://www.w3.org/1998/Math/MathML"><mi>&#x3B1;</mi><mo>&lt;</mo><mn>2</mn></math>"#;
    #[test]
    fn math_is_reconstructed_from_decoded_xml_and_has_private_markup() {
        let value = ValidatedMath::validate("body", "m0", SVG, MATHML).unwrap();
        assert!(value.mathml().contains("<mi>α</mi>"));
        assert!(value.mathml().contains("<mo>&lt;</mo>"));
        assert_eq!(value.ids(), &BTreeSet::from(["body-m0-i0".into()]));
        assert_eq!(value.references(), value.ids());
        assert!(!value.svg().contains("/>"));
    }
    #[test]
    fn active_xml_and_unknown_profiles_are_rejected() {
        for svg in [
            SVG.replace("<g ", "<g onclick=\"alert(1)\" "),
            SVG.replace("<defs>", "<defs><script>alert(1)</script>"),
            SVG.replace("<defs>", "<defs><foreignObject><p>HTML</p></foreignObject>"),
            SVG.replace("#body-m0-i0", "https://example.org/x"),
            SVG.replace("#body-m0-i0", "#notes-m0-i0"),
            SVG.replace("body-m0-i0", "body-m1-i0"),
            SVG.replace("width=\"1ex\"", "width=\"NaN\""),
            SVG.replace("<g ", "<g style=\"background:url(https://example.org)\" "),
            SVG.replace("<g ", "<g style=\"overflow: hidden\" "),
            SVG.replace("<g ", "<g xmlns=\"http://www.w3.org/1999/xhtml\" "),
            SVG.replace("<g ", "<g xmlns:evil=\"https://example.org\" "),
            format!("<!DOCTYPE svg>{SVG}"),
            format!("<?xml version=\"1.0\"?>{SVG}"),
            SVG.replace("<defs>", "<defs><!--comment-->"),
            SVG.replace("<defs>", "<defs><![CDATA[raw]]>"),
        ] {
            assert!(
                ValidatedMath::validate("body", "m0", &svg, MATHML).is_err(),
                "accepted {svg}"
            );
        }
        for mathml in [
            MATHML.replace("<mi>", "<mi id=\"body-m0-i1\">"),
            MATHML
                .replace("<mi>", "<annotation-xml encoding=\"text/html\"><mi>")
                .replace("</mi>", "</mi></annotation-xml>"),
            MATHML.replace("<mi>", "<mi href=\"javascript:alert(1)\">"),
        ] {
            assert!(ValidatedMath::validate("body", "m0", SVG, &mathml).is_err());
        }
    }
    #[test]
    fn math_depth_bytes_and_node_limits_are_enforced() {
        let deep = format!(
            "<math xmlns=\"http://www.w3.org/1998/Math/MathML\">{}x{}</math>",
            "<mrow>".repeat(65),
            "</mrow>".repeat(65)
        );
        assert!(ValidatedMath::validate("body", "m0", SVG, &deep).is_err());
        assert!(ValidatedMath::validate("body", "m0", &" ".repeat(XML_BYTES + 1), MATHML).is_err());
        let wide = format!(
            "<math xmlns=\"http://www.w3.org/1998/Math/MathML\">{}</math>",
            "<mi>x</mi>".repeat(XML_NODES as usize)
        );
        assert!(ValidatedMath::validate("body", "m0", SVG, &wide).is_err());
    }
    #[test]
    fn rich_text_escapes_text_and_rejects_active_links_and_deep_trees() {
        let value = ValidatedRichText::validate(vec![
            RichInline::Emphasis {
                children: vec![RichInline::Text {
                    text: "<script>&\"".into(),
                }],
            },
            RichInline::Link {
                href: "https://example.org/?x=1&y=2".into(),
                children: vec![RichInline::Text {
                    text: "link".into(),
                }],
            },
        ])
        .unwrap();
        assert!(value.html().contains("<em>&lt;script&gt;&amp;&quot;</em>"));
        assert!(value.html().contains("x=1&amp;y=2"));
        for href in [
            "javascript:alert(1)",
            "data:text/html,x",
            "//example.org/x",
            "relative",
            "mailto:a@example.org",
        ] {
            assert!(
                ValidatedRichText::validate(vec![RichInline::Link {
                    href: href.into(),
                    children: vec![]
                }])
                .is_err()
            );
        }
        let mut tree = RichInline::Text { text: "x".into() };
        for _ in 0..34 {
            tree = RichInline::Strong {
                children: vec![tree],
            };
        }
        assert!(ValidatedRichText::validate(vec![tree]).is_err());
        assert!(
            ValidatedRichText::validate(vec![RichInline::Text {
                text: "x".repeat(RICH_BYTES + 1)
            }])
            .is_err()
        );
        assert!(
            ValidatedRichText::validate(vec![RichInline::Text { text: "".into() }; RICH_NODES + 1])
                .is_err()
        );
    }
}
