//! Acquire selected slide resources and produce a fixed, offline HTML bundle.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use adocweave_core::output::diagnostics::{Diagnostic, Severity};
use adocweave_core::preprocess::{ExpandedRange, PreprocessedAnalysis};
use adocweave_core::resolution::{MediaType, RenderInputs, ResolvedResource, ResourcePurpose};
use adocweave_core::semantic::{self, Block, Inline, ReferenceDestination, SemanticNode};
use adocweave_core::{NeverCancel, OutputLimits};
use adocweave_project::{
    BundleFile, BundleMediaType, ProjectAuthority, ProjectResourceLimits, ProjectTargetResult,
};

use super::{Audience, Deck, problem, unsupported_fragment_name};
use crate::cli_error::CliError;

const SPEAKER_SCRIPT_HASH: &str = "sha256-GzCveToXhSIzS3M5eQeRm3McVRB7cYneYKwTkzA3wDk=";

pub(crate) struct GeneratedBundle {
    pub(crate) files: Vec<BundleFile>,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

/// HTTP servers add frame-ancestors, which browsers ignore in a meta policy.
pub(crate) fn content_security_policy(audience: Audience) -> String {
    let speaker = if audience == Audience::Presenter {
        format!(" '{SPEAKER_SCRIPT_HASH}'")
    } else {
        String::new()
    };
    format!(
        "default-src 'none'; base-uri 'none'; object-src 'none'; script-src 'self'{speaker}; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self'; media-src 'self'; connect-src 'self'; frame-src 'self'; form-action 'none'"
    )
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn static_file(path: &str, media_type: BundleMediaType, bytes: &[u8]) -> BundleFile {
    BundleFile {
        path: path.to_owned(),
        media_type,
        bytes: bytes.to_vec(),
    }
}

fn fixed_files(audience: Audience) -> Vec<BundleFile> {
    let mut files = vec![
        static_file(
            "assets/reveal.js",
            BundleMediaType::JavaScript,
            include_bytes!("../../assets/revealjs/reveal.js"),
        ),
        static_file(
            "assets/reveal.css",
            BundleMediaType::Css,
            include_bytes!("../../assets/revealjs/reveal.css"),
        ),
        static_file(
            "assets/reset.css",
            BundleMediaType::Css,
            include_bytes!("../../assets/revealjs/reset.css"),
        ),
        static_file(
            "assets/theme.css",
            BundleMediaType::Css,
            include_bytes!("../../assets/slides/theme.css"),
        ),
        static_file(
            "assets/bootstrap.js",
            BundleMediaType::JavaScript,
            include_bytes!("../../assets/slides/bootstrap.js"),
        ),
        static_file(
            "licenses/revealjs.txt",
            BundleMediaType::Text,
            include_bytes!("../../assets/revealjs/LICENSE.revealjs.txt"),
        ),
        static_file(
            "licenses/fitty.txt",
            BundleMediaType::Text,
            include_bytes!("../../assets/revealjs/LICENSE.fitty.txt"),
        ),
    ];
    let mut notice = "reveal.js 6.0.2 and its bundled fitty 2.4.2 are distributed under their accompanying MIT licenses.\n".to_owned();
    if audience == Audience::Presenter {
        files.push(static_file(
            "assets/notes.js",
            BundleMediaType::JavaScript,
            include_bytes!("../../assets/revealjs/notes.js"),
        ));
        files.push(static_file(
            "licenses/marked.txt",
            BundleMediaType::Text,
            include_bytes!("../../assets/revealjs/LICENSE.marked.txt"),
        ));
        notice.push_str(
            "The speaker notes plugin bundles marked 17.0.5 under its accompanying MIT license.\n",
        );
    }
    files.push(static_file(
        "licenses/NOTICE.txt",
        BundleMediaType::Text,
        notice.as_bytes(),
    ));
    files
}

fn page(deck: &Deck<'_>, html: &str, audience: Audience) -> String {
    let title = deck
        .groups
        .first()
        .and_then(|group| group.slides.first())
        .map_or("Slides", |slide| slide.title.as_str());
    let audience_name = if audience == Audience::Presenter {
        "presenter"
    } else {
        "public"
    };
    let notes = if audience == Audience::Presenter {
        "<script src=\"assets/notes.js\"></script>\n"
    } else {
        ""
    };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta http-equiv=\"Content-Security-Policy\" content=\"{}\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n<link rel=\"stylesheet\" href=\"assets/reset.css\">\n<link rel=\"stylesheet\" href=\"assets/reveal.css\">\n<link rel=\"stylesheet\" href=\"assets/theme.css\">\n</head>\n<body data-audience=\"{audience_name}\">\n{html}<script src=\"assets/reveal.js\"></script>\n{notes}<script src=\"assets/bootstrap.js\"></script>\n</body>\n</html>\n",
        escape(&content_security_policy(audience)),
        escape(title)
    )
}

fn source_base(
    preprocessed: &PreprocessedAnalysis,
    target: &ProjectTargetResult,
    range: adocweave_core::text::TextRange,
    primary_base: &Path,
) -> PathBuf {
    let source_id = preprocessed
        .document
        .origins_for_range(ExpandedRange::new(range))
        .into_iter()
        .find_map(|origin| origin.source_id);
    source_id
        .as_ref()
        .and_then(|id| {
            target
                .resources
                .iter()
                .find(|resource| &resource.source_id == id)
        })
        .and_then(|resource| resource.path.parent())
        .unwrap_or(primary_base)
        .to_owned()
}

fn image_type(bytes: &[u8]) -> Option<(BundleMediaType, &'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some((BundleMediaType::Png, "png", "image/png"))
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some((BundleMediaType::Jpeg, "jpg", "image/jpeg"))
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some((BundleMediaType::Gif, "gif", "image/gif"))
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some((BundleMediaType::Webp, "webp", "image/webp"))
    } else {
        None
    }
}

pub(crate) fn build(
    preprocessed: &PreprocessedAnalysis,
    target: &ProjectTargetResult,
    authority: &ProjectAuthority,
    primary_base: &Path,
    audience: Audience,
    slides_helper: Option<&Path>,
    remaining_resources: ProjectResourceLimits,
) -> Result<GeneratedBundle, CliError> {
    let analysis = &preprocessed.analysis;
    let deck = Deck::compile(analysis.document());
    let mut diagnostics = analysis.diagnostics().to_vec();
    diagnostics.extend(deck.diagnostics.clone());
    if let Some(name) = target
        .config
        .config
        .analysis()
        .attributes
        .keys()
        .find(|name| name.starts_with("revealjs_") || name.as_str() == "notitle")
    {
        return Err(CliError::Usage(format!(
            "unsupported reveal.js configuration attribute: {name}"
        )));
    }
    if !target.config.config.stylesheet_files().is_empty()
        || !target
            .config
            .config
            .html_policy()
            .stylesheets
            .sources
            .is_empty()
    {
        return Err(CliError::Usage(
            "HTML stylesheets are not supported by revealjs output".to_owned(),
        ));
    }
    let visible = |range| {
        deck.contains_body_range(range)
            || (audience == Audience::Presenter && deck.contains_note_range(range))
    };
    let targets = analysis.document().reference_targets();
    for reference in analysis
        .references()
        .iter()
        .filter(|reference| deck.contains_body_range(reference.range))
    {
        if let ReferenceDestination::Local { anchor, .. } = &reference.authored_destination
            && targets
                .iter()
                .any(|target| target.id == *anchor && deck.contains_note_range(target.target_range))
        {
            problem(
                &mut diagnostics,
                "slides-note-only-reference",
                "slide body cannot reference a target defined only in presenter notes",
                reference.range,
            );
        }
    }
    semantic::walk(analysis.document(), |node| {
        if let SemanticNode::Inline(Inline::Macro(node)) = node
            && node.kind == semantic::StandardMacroKind::Footnote
            && visible(node.range)
        {
            problem(
                &mut diagnostics,
                "slides-footnote-unavailable",
                "scoped slide footnotes are not connected in this build",
                node.range,
            );
        }
        if let SemanticNode::Inline(Inline::Text(text)) = node
            && visible(text.range)
        {
            let raw = &analysis.source()
                [text.range.start().to_u32() as usize..text.range.end().to_u32() as usize];
            for (offset, _) in raw.match_indices('[') {
                if offset > 0 && raw.as_bytes()[offset - 1] == b'\\' {
                    continue;
                }
                let Some(end) = raw[offset..].find(']') else {
                    continue;
                };
                let marker = &raw[offset + 1..offset + end];
                let after = text.range.start().to_u32() as usize + offset + end + 1;
                if analysis.source().as_bytes().get(after) == Some(&b'#')
                    && marker
                        .strip_prefix(['.', '%'])
                        .is_some_and(unsupported_fragment_name)
                {
                    problem(
                        &mut diagnostics,
                        "slides-inline-step",
                        "inline fragment presentation is not supported",
                        text.range,
                    );
                }
            }
        }
        let math_range = match node {
            SemanticNode::Block(Block::Math(math)) => Some(math.range),
            SemanticNode::Inline(Inline::Formula(formula)) => Some(formula.range),
            _ => None,
        };
        if let Some(range) = math_range.filter(|range| visible(*range)) {
            problem(
                &mut diagnostics,
                "slides-math-unavailable",
                "verified slide math rendering is not connected in this build",
                range,
            );
        }
    });
    for citation in analysis
        .citations()
        .into_iter()
        .filter(|citation| visible(citation.range))
    {
        problem(
            &mut diagnostics,
            "slides-citation-unavailable",
            "verified slide citation rendering is not connected in this build",
            citation.range,
        );
    }
    let _ = slides_helper; // Host integration consumes this path in the next integration step.
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        return Ok(GeneratedBundle {
            files: Vec::new(),
            diagnostics,
        });
    }
    let mut requested = Vec::new();
    for reference in analysis
        .resources()
        .iter()
        .filter(|reference| visible(reference.owner_range()))
    {
        let authored = reference.target();
        if reference.purpose() != ResourcePurpose::Image
            || reference.target_expansion_error().is_some()
            || Path::new(authored).is_absolute()
            || authored.contains(':')
            || authored.contains('\\')
        {
            problem(
                &mut diagnostics,
                "slides-unsupported-resource",
                "slides require a local image path inside the configured resource roots",
                reference.range(),
            );
            continue;
        }
        let base = source_base(preprocessed, target, reference.range(), primary_base);
        let path = adocweave_project::absolute_path(&base, Path::new(authored))
            .map_err(CliError::Project)?;
        requested.push((reference, path));
    }
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        return Ok(GeneratedBundle {
            files: Vec::new(),
            diagnostics,
        });
    }
    let paths = requested
        .iter()
        .map(|(_, path)| path.clone())
        .collect::<Vec<_>>();
    let configured_roots = target.config.config.resource_roots();
    let default_roots = vec![primary_base.to_owned()];
    let roots = if configured_roots.is_empty() {
        &default_roots
    } else {
        configured_roots
    };
    let acquired = authority
        .read_binary_resources(roots, &paths, remaining_resources, &NeverCancel)
        .map_err(CliError::Project)?;
    let acquired = acquired
        .into_iter()
        .map(|resource| (resource.path.clone(), resource))
        .collect::<BTreeMap<_, _>>();
    let mut assets = BTreeMap::new();
    let mut body = Vec::new();
    let mut notes = Vec::new();
    for (reference, path) in requested {
        let mut resource = acquired[&path].clone();
        let kind = if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
        {
            match super::svg::validate(&resource.bytes) {
                Ok(bytes) => {
                    resource.bytes = bytes;
                    Some((BundleMediaType::Svg, "svg", "image/svg+xml"))
                }
                Err(message) => {
                    problem(
                        &mut diagnostics,
                        "slides-unsafe-svg",
                        message,
                        reference.range(),
                    );
                    continue;
                }
            }
        } else {
            image_type(&resource.bytes)
        };
        let Some((media_type, extension, mime)) = kind else {
            problem(
                &mut diagnostics,
                "slides-unsupported-image",
                "image must be a supported SVG or have a PNG, JPEG, GIF or WebP signature",
                reference.range(),
            );
            continue;
        };
        let href = format!("assets/{}.{}", resource.sha256(), extension);
        assets
            .entry(href.clone())
            .or_insert_with(|| static_file(&href, media_type, &resource.bytes));
        let resolved = ResolvedResource::resolved(
            reference.range(),
            href,
            MediaType::parse(mime).expect("fixed image MIME"),
            Some(resource.bytes.len() as u64),
        );
        if deck.contains_body_range(reference.owner_range()) {
            body.push(resolved);
        } else {
            notes.push(resolved);
        }
    }
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        return Ok(GeneratedBundle {
            files: Vec::new(),
            diagnostics,
        });
    }
    let mut policy = target.config.config.html_policy().clone();
    // The only resource resolutions here are our verified, digest-named files.
    policy.active_urls.allow_resolved_relative = true;
    let rendered = deck
        .render(
            audience,
            &policy,
            &RenderInputs::default().with_resources(body),
            &RenderInputs::default().with_resources(notes),
            OutputLimits::default(),
        )
        .map_err(|error| CliError::Slides(error.to_string()))?;
    diagnostics.extend(
        rendered
            .diagnostics
            .into_iter()
            .filter(|diagnostic| {
                !diagnostics
                    .iter()
                    .any(|existing| existing.id == diagnostic.id)
            })
            .collect::<Vec<_>>(),
    );
    let mut files = fixed_files(audience);
    files.extend(assets.into_values());
    files.push(static_file(
        "index.html",
        BundleMediaType::Html,
        page(&deck, &rendered.html, audience).as_bytes(),
    ));
    adocweave_core::output::diagnostics::sort_diagnostics(&mut diagnostics);
    Ok(GeneratedBundle { files, diagnostics })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_pages_have_early_csp_and_no_inline_script_or_network_dependency() {
        let analysis = adocweave_core::Engine::new(Default::default())
            .analyze("= Talk\n\n== Slide\n\nText.\n")
            .unwrap();
        let deck = Deck::compile(analysis.document());
        let rendered = deck
            .render(
                Audience::Public,
                &Default::default(),
                &Default::default(),
                &Default::default(),
                Default::default(),
            )
            .unwrap();
        for audience in [Audience::Public, Audience::Presenter] {
            let html = page(&deck, &rendered.html, audience);
            assert!(
                html.find("Content-Security-Policy").unwrap() < html.find("stylesheet").unwrap()
            );
            assert!(!html.contains("<script>"));
            assert!(!content_security_policy(audience).contains("unsafe-eval"));
            assert!(
                !content_security_policy(audience).contains("script-src 'self' 'unsafe-inline'")
            );
            assert_eq!(html.contains("notes.js"), audience == Audience::Presenter);
            assert_eq!(
                fixed_files(audience)
                    .iter()
                    .any(|file| file.path == "assets/notes.js"),
                audience == Audience::Presenter
            );
        }
    }
}
