//! Package only freshly generated public slides; this is not an HTML importer.

use std::collections::BTreeSet;
use std::ops::Range;

use adocweave_project::BundleMediaType;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest as _, Sha256};

use super::Audience;
use super::bundle::{GeneratedBundle, content_security_policy, escape};
use crate::cli_error::CliError;

const LICENSE_SCRIPT: &str = r#"
document.addEventListener("click", event => {
  const link = event.target.closest("a[data-slides-license]");
  if (!link) return;
  event.preventDefault();
  const notice = document.getElementById(link.getAttribute("href").slice(1));
  notice.closest("details").open = true;
  notice.scrollIntoView();
});
"#;
const LICENSE_CSS: &str = r#"
body > .slides-licenses {
  position: fixed;
  top: 0;
  left: 0;
  z-index: 1000;
  max-width: 100%;
  max-height: 75vh;
  overflow: auto;
  background: var(--adocweave-surface);
  color: var(--adocweave-muted);
  font: 14px/1.4 var(--adocweave-font-family);
  padding: .3em .6em;
  box-sizing: border-box;
}
body > .slides-licenses summary {
  position: sticky;
  top: 0;
  z-index: 1;
  background: var(--adocweave-surface);
  cursor: pointer;
}
body > .slides-licenses pre {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  scroll-margin-top: 2em;
}
body > .slides-licenses h2 { font-size: 1em; }
@media print { body > .slides-licenses { display: none; } }
"#;

fn invalid(message: impl Into<String>) -> CliError {
    CliError::Slides(format!(
        "cannot package single-file slides: {}",
        message.into()
    ))
}

fn base64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

fn data_url(media_type: BundleMediaType, bytes: &[u8]) -> String {
    format!(
        "data:{};base64,{}",
        media_type.content_type(),
        base64(bytes)
    )
}

fn inline_script(text: &str) -> Result<(String, String), CliError> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let lower = text.to_ascii_lowercase();
    if lower.contains("</script") || lower.contains("<!--") || lower.contains("<script") {
        return Err(invalid(
            "embedded JavaScript contains an HTML script delimiter",
        ));
    }
    let hash = base64(&Sha256::digest(text.as_bytes()));
    Ok((
        format!("<script>{text}</script>"),
        format!("'sha256-{hash}'"),
    ))
}

/// The caller has already selected public content and checked generation errors.
/// Replacements refer only to markup emitted by `bundle::page` and the renderer.
pub(crate) fn render(bundle: &GeneratedBundle, limit: u32) -> Result<String, CliError> {
    let mut paths = BTreeSet::new();
    for file in &bundle.files {
        if !paths.insert(file.path.as_str()) {
            return Err(invalid("duplicate asset path"));
        }
    }
    let page = bundle
        .files
        .iter()
        .find(|file| file.path == "index.html" && file.media_type == BundleMediaType::Html)
        .ok_or_else(|| invalid("index.html is missing"))?;
    let html = std::str::from_utf8(&page.bytes).map_err(|_| invalid("index.html is not UTF-8"))?;
    let metadata = bundle
        .files
        .iter()
        .find(|file| file.path == "slides.json" && file.media_type == BundleMediaType::Json)
        .ok_or_else(|| invalid("slide metadata is missing"))?;
    let metadata: serde_json::Value =
        serde_json::from_slice(&metadata.bytes).map_err(|_| invalid("invalid slide metadata"))?;
    if metadata != serde_json::json!({"schema_version":1,"audience":"public"})
        || !html.contains("<body data-audience=\"public\"")
        || html.contains("data-preview=\"true\"")
    {
        return Err(invalid(
            "only public slides without live preview can be packaged",
        ));
    }
    let mut replacements: Vec<String> = Vec::new();
    let mut edits: Vec<(Range<usize>, usize)> = Vec::new();
    let mut final_size = html.len() as u64;
    let mut replace = |needle: &str, replacement: String, required: bool| -> Result<(), CliError> {
        let ranges = html
            .match_indices(needle)
            .map(|(start, _)| start..start + needle.len())
            .collect::<Vec<_>>();
        if required && ranges.is_empty() {
            return Err(invalid(format!(
                "generated asset reference is missing: {needle}"
            )));
        }
        final_size = final_size
            .saturating_sub((ranges.len() * needle.len()) as u64)
            .saturating_add((ranges.len() as u64).saturating_mul(replacement.len() as u64));
        let index = replacements.len();
        replacements.push(replacement);
        edits.extend(ranges.into_iter().map(|range| (range, index)));
        Ok(())
    };
    let mut hashes = Vec::new();
    let mut notices = String::from(
        "<details class=\"slides-licenses\"><summary>Licenses and notices</summary>\n",
    );
    let mut ids = BTreeSet::new();
    let mut scripts = BTreeSet::new();
    for file in &bundle.files {
        let path = &file.path;
        match file.media_type {
            BundleMediaType::Html if path == "index.html" => {}
            BundleMediaType::Json if path == "slides.json" => {}
            BundleMediaType::JavaScript
                if matches!(path.as_str(), "assets/reveal.js" | "assets/bootstrap.js") =>
            {
                scripts.insert(path.as_str());
                let text = std::str::from_utf8(&file.bytes)
                    .map_err(|_| invalid("JavaScript is not UTF-8"))?;
                let text = if path == "assets/bootstrap.js" {
                    format!("{text}\n{LICENSE_SCRIPT}")
                } else {
                    text.to_owned()
                };
                let (script, hash) = inline_script(&text)?;
                hashes.push(hash);
                replace(
                    &format!("<script src=\"{}\"></script>", escape(path)),
                    script,
                    true,
                )?;
            }
            BundleMediaType::Css if path.starts_with("assets/") => {
                replace(
                    &format!("<link rel=\"stylesheet\" href=\"{}\">", escape(path)),
                    format!(
                        "<link rel=\"stylesheet\" href=\"{}\">",
                        data_url(file.media_type, &file.bytes)
                    ),
                    true,
                )?;
            }
            BundleMediaType::Png
            | BundleMediaType::Jpeg
            | BundleMediaType::Gif
            | BundleMediaType::Webp
            | BundleMediaType::Svg
                if path.starts_with("assets/") =>
            {
                replace(
                    &format!("src=\"{}\"", escape(path)),
                    format!("src=\"{}\"", data_url(file.media_type, &file.bytes)),
                    true,
                )?;
            }
            BundleMediaType::Text
                if path.starts_with("licenses/") && path != "licenses/marked.txt" =>
            {
                let text = std::str::from_utf8(&file.bytes)
                    .map_err(|_| invalid("license is not UTF-8"))?;
                let mut id = format!("adocweave-license-{}", ids.len() + 1);
                while html.contains(&format!("id=\"{id}\"")) || !ids.insert(id.clone()) {
                    id.push('_');
                }
                replace(
                    &format!("href=\"{}\"", escape(path)),
                    format!("href=\"#{id}\" data-slides-license"),
                    false,
                )?;
                notices.push_str(&format!(
                    // HTML drops this added LF, preserving any leading LF in the notice.
                    "<section><h2>{}</h2><pre id=\"{id}\">\n{}</pre></section>\n",
                    escape(path),
                    escape(text).replace('\r', "&#13;").replace('\n', "&#10;")
                ));
                if notices.len() as u64 > u64::from(limit) {
                    return Err(CliError::OutputLimit {
                        limit,
                        actual: notices.len() as u64,
                    });
                }
            }
            _ => return Err(invalid(format!("unsupported asset: {path}"))),
        }
    }
    if scripts.len() != 2 {
        return Err(invalid("required presentation scripts are missing"));
    }
    notices.push_str("</details>\n");
    replace("</body>", format!("{notices}</body>"), true)?;
    replace(
        "</head>",
        format!(
            "<link rel=\"stylesheet\" href=\"{}\">\n</head>",
            data_url(BundleMediaType::Css, LICENSE_CSS.as_bytes())
        ),
        true,
    )?;
    let policy = format!(
        "default-src 'none'; base-uri 'none'; object-src 'none'; script-src {}; style-src data: 'unsafe-inline'; img-src data:; font-src data:; form-action 'none'",
        hashes.join(" ")
    );
    replace(
        &format!(
            "<meta http-equiv=\"Content-Security-Policy\" content=\"{}\">",
            escape(&content_security_policy(Audience::Public))
        ),
        format!(
            "<meta http-equiv=\"Content-Security-Policy\" content=\"{}\">",
            escape(&policy)
        ),
        true,
    )?;
    if final_size > u64::from(limit) {
        return Err(CliError::OutputLimit {
            limit,
            actual: final_size,
        });
    }
    edits.sort_by_key(|(range, _)| range.start);
    let mut output = String::with_capacity(final_size as usize);
    let mut previous = 0;
    for (range, index) in edits {
        if range.start < previous {
            return Err(invalid("overlapping generated asset references"));
        }
        output.push_str(&html[previous..range.start]);
        output.push_str(&replacements[index]);
        previous = range.end;
    }
    output.push_str(&html[previous..]);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use adocweave_project::BundleFile;

    fn file(path: &str, media_type: BundleMediaType, text: &[u8]) -> BundleFile {
        BundleFile {
            path: path.to_owned(),
            media_type,
            bytes: text.to_vec(),
        }
    }

    fn fixture() -> GeneratedBundle {
        let html = format!(
            r#"<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="{}"><link rel="stylesheet" href="assets/test.css"></head><body data-audience="public"><img src="assets/test.png"><img src="assets/test.png"><a href="licenses/citations-license.txt">License</a><script src="assets/reveal.js"></script><script src="assets/bootstrap.js"></script></body></html>"#,
            escape(&content_security_policy(Audience::Public))
        );
        GeneratedBundle {
            files: vec![
                file("index.html", BundleMediaType::Html, html.as_bytes()),
                file(
                    "slides.json",
                    BundleMediaType::Json,
                    br#"{"schema_version":1,"audience":"public"}"#,
                ),
                file(
                    "assets/reveal.js",
                    BundleMediaType::JavaScript,
                    b"const reveal = true;\r\n",
                ),
                file(
                    "assets/bootstrap.js",
                    BundleMediaType::JavaScript,
                    b"const bootstrap = true;",
                ),
                file(
                    "assets/test.css",
                    BundleMediaType::Css,
                    b"p::after { content: '</style><script>bad()</script>'; }",
                ),
                file(
                    "assets/test.png",
                    BundleMediaType::Png,
                    b"\x89PNG\r\n\x1a\n",
                ),
                file(
                    "licenses/citations-license.txt",
                    BundleMediaType::Text,
                    b"\nLicense <script> & text\r\n",
                ),
            ],
            diagnostics: vec![],
            observations: vec![],
        }
    }

    #[test]
    fn assets_are_embedded_without_rewriting_newly_inserted_content() {
        let bundle = fixture();
        let output = render(&bundle, 100_000).unwrap();
        assert!(!output.contains("src=\"assets/"));
        assert!(!output.contains("href=\"assets/"));
        assert!(!output.contains("href=\"licenses/"));
        assert!(!output.contains("</style><script>bad()"));
        let image = data_url(BundleMediaType::Png, &bundle.files[5].bytes);
        assert_eq!(output.matches(&image).count(), 2);
        let css = data_url(BundleMediaType::Css, &bundle.files[4].bytes);
        assert!(output.contains(&css));
        assert!(output.contains("&#10;License &lt;script&gt; &amp; text&#13;&#10;"));
        assert!(output.contains("href=\"#adocweave-license-1\" data-slides-license"));
        for script in output.split("<script>").skip(1) {
            let body = script.split_once("</script>").unwrap().0;
            assert!(!body.contains('\r'));
            assert!(output.contains(&format!(
                "sha256-{}",
                base64(&Sha256::digest(body.as_bytes()))
            )));
        }
        assert!(!output.contains("'unsafe-eval'"));
        assert!(!output.contains("&#39;self&#39;"));
    }

    #[test]
    fn final_size_counts_repeated_images_and_notices() {
        let bundle = fixture();
        let output = render(&bundle, 100_000).unwrap();
        assert_eq!(render(&bundle, output.len() as u32).unwrap(), output);
        assert!(matches!(
            render(&bundle, output.len() as u32 - 1),
            Err(CliError::OutputLimit { .. })
        ));
    }

    #[test]
    fn presenter_preview_and_unknown_assets_are_rejected() {
        let mut bundle = fixture();
        bundle.files[1].bytes = br#"{"schema_version":1,"audience":"presenter"}"#.to_vec();
        assert!(render(&bundle, 100_000).is_err());
        let mut bundle = fixture();
        bundle.files[0].bytes = String::from_utf8(bundle.files[0].bytes.clone())
            .unwrap()
            .replace(
                "data-audience=\"public\"",
                "data-audience=\"public\" data-preview=\"true\"",
            )
            .into_bytes();
        assert!(render(&bundle, 100_000).is_err());
        for (path, kind) in [
            ("assets/notes.js", BundleMediaType::JavaScript),
            ("licenses/marked.txt", BundleMediaType::Text),
            ("assets/font.woff2", BundleMediaType::Woff2),
            ("assets/unused.png", BundleMediaType::Png),
        ] {
            let mut bundle = fixture();
            bundle
                .files
                .push(file(path, kind, b"private or unsupported"));
            assert!(render(&bundle, 100_000).is_err(), "{path}");
        }
    }

    #[test]
    fn script_delimiters_fail_and_notice_ids_avoid_authored_ids() {
        for source in ["</script>", "</SCRIPT ", "<!--", "<ScRiPt>"] {
            assert!(inline_script(source).is_err(), "{source}");
        }
        let mut bundle = fixture();
        bundle.files[0].bytes = String::from_utf8(bundle.files[0].bytes.clone())
            .unwrap()
            .replace("<img", "<span id=\"adocweave-license-1\"></span><img")
            .into_bytes();
        let output = render(&bundle, 100_000).unwrap();
        assert!(output.contains("href=\"#adocweave-license-1_\""));
    }
}
