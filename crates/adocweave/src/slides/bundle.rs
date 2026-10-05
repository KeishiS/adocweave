//! Acquire selected slide resources and produce a fixed, offline HTML bundle.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use adocweave_core::output::diagnostics::{Diagnostic, Severity};
use adocweave_core::preprocess::{ExpandedRange, PreprocessedAnalysis};
use adocweave_core::resolution::{MediaType, ResolvedResource, ResourcePurpose};
use adocweave_core::semantic::{self, Inline, ReferenceDestination, SemanticNode};
use adocweave_core::{CancellationCheck, OutputLimits};
use adocweave_project::{
    BundleFile, BundleMediaType, BundleSnapshot, ProjectAuthority, ProjectObservationCandidate,
    ProjectResourceLimits, ProjectTargetResult,
};

use super::{Audience, Deck, problem, unsupported_fragment_name};
use crate::cli_error::CliError;

const SPEAKER_SCRIPT_HASH: &str = "sha256-GzCveToXhSIzS3M5eQeRm3McVRB7cYneYKwTkzA3wDk=";

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SlideMetadata {
    schema_version: u32,
    audience: Audience,
}

pub(crate) fn audience_from_bundle(bundle: &BundleSnapshot) -> Result<Audience, CliError> {
    if bundle.file("index.html").is_none_or(|file| {
        file.media_type != BundleMediaType::Html || std::str::from_utf8(&file.bytes).is_err()
    }) {
        return Err(CliError::Slides(
            "managed slide bundle must contain a UTF-8 index.html page".to_owned(),
        ));
    }
    let file = bundle
        .file("slides.json")
        .filter(|file| file.media_type == BundleMediaType::Json)
        .ok_or_else(|| {
            CliError::Slides("slide metadata is missing; regenerate the slide bundle".to_owned())
        })?;
    let metadata: SlideMetadata = serde_json::from_slice(&file.bytes)
        .map_err(|error| CliError::Slides(format!("invalid slide metadata: {error}")))?;
    if metadata.schema_version != 1 {
        return Err(CliError::Slides(
            "unsupported slide metadata version; regenerate the slide bundle".to_owned(),
        ));
    }
    Ok(metadata.audience)
}

pub(crate) struct GeneratedBundle {
    pub(crate) files: Vec<BundleFile>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) observations: Vec<ProjectObservationCandidate>,
}

pub(crate) struct Options<'a> {
    audience: Audience,
    helper: Option<&'a Path>,
    data: &'a crate::arguments::SlidesData,
    preview: bool,
}

impl<'a> Options<'a> {
    pub(crate) fn convert(
        audience: Audience,
        helper: Option<&'a Path>,
        data: &'a crate::arguments::SlidesData,
    ) -> Self {
        Self {
            audience,
            helper,
            data,
            preview: false,
        }
    }
    pub(crate) fn with_preview(mut self) -> Self {
        self.preview = true;
        self
    }
}

fn host_error(
    error: super::helper::HostError,
    diagnostics: &mut Vec<Diagnostic>,
    observations: &[ProjectObservationCandidate],
) -> Result<(), CliError> {
    if let Some(range) = error.range {
        problem(diagnostics, error.code, &error.message, range);
        Ok(())
    } else {
        Err(CliError::SlidesResources {
            message: error.to_string(),
            observations: observations.to_vec(),
        })
    }
}

fn helper_notices(notices: &super::helper::protocol::Notices, files: &mut Vec<BundleFile>) {
    if let Some(math) = &notices.math {
        for (path, text) in [
            ("licenses/math-font-attribution.txt", &math.font_attribution),
            ("licenses/math-font-license.txt", &math.font_license),
            ("licenses/math-font-lppl.txt", &math.lppl_license),
            ("licenses/mathjax-license.txt", &math.mathjax_license),
        ] {
            files.push(static_file(path, BundleMediaType::Text, text.as_bytes()));
        }
    }
    if let Some(citations) = &notices.citations {
        for (path, text) in [
            ("licenses/citations-attribution.txt", &citations.attribution),
            ("licenses/citations-license.txt", &citations.license),
        ] {
            files.push(static_file(path, BundleMediaType::Text, text.as_bytes()));
        }
    }
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

pub(super) fn escape(value: &str) -> String {
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

fn fixed_files(audience: Audience, preview: bool) -> Vec<BundleFile> {
    let mut files = vec![
        static_file(
            "slides.json",
            BundleMediaType::Json,
            &serde_json::to_vec(&SlideMetadata {
                schema_version: 1,
                audience,
            })
            .expect("slide metadata contains only serializable fields"),
        ),
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
            "assets/content.css",
            BundleMediaType::Css,
            include_bytes!("../../assets/slides/content.css"),
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
    if preview {
        files.push(static_file(
            "assets/preview.js",
            BundleMediaType::JavaScript,
            include_bytes!("../../assets/slides/preview.js"),
        ));
    }
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

fn parse_aspect_ratio(value: &str) -> Option<String> {
    let (width, height) = value.trim().split_once(':')?;
    let width = width.trim().parse::<u32>().ok()?;
    let height = height.trim().parse::<u32>().ok()?;
    if width == 0 || height == 0 || !(0.25..=4.0).contains(&(width as f64 / height as f64)) {
        return None;
    }
    let (mut divisor, mut remainder) = (width, height);
    while remainder != 0 {
        (divisor, remainder) = (remainder, divisor % remainder);
    }
    Some(format!("{}:{}", width / divisor, height / divisor))
}

fn resolve_aspect_ratio(
    analysis: &adocweave_core::Analysis,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Option<String>, CliError> {
    let Some(attribute) = analysis
        .attribute_environment()
        .resolve_at("slides-aspect-ratio", analysis.document().header().end)
    else {
        return Ok(None);
    };
    if matches!(attribute.value, Ok(None)) {
        return Ok(None);
    }
    if let Ok(Some(value)) = attribute.value {
        if value.trim() == "auto" {
            return Ok(None);
        }
        if let Some(ratio) = parse_aspect_ratio(value) {
            return Ok(Some(ratio));
        }
    }
    let message = "slides-aspect-ratio must be auto or WIDTH:HEIGHT, using positive integers and a ratio between 1:4 and 4:1";
    if let Some(binding) = attribute.binding {
        problem(
            diagnostics,
            "slides-invalid-aspect-ratio",
            message,
            binding.occurrence().range,
        );
        Ok(None)
    } else {
        Err(CliError::Usage(message.to_owned()))
    }
}

struct PageOptions<'a> {
    audience: Audience,
    language: &'a str,
    aspect_ratio: Option<&'a str>,
    preview: bool,
}

fn page(
    deck: &Deck<'_>,
    html: &str,
    options: PageOptions<'_>,
    attribution: Option<&str>,
    styles: &[String],
) -> String {
    let PageOptions {
        audience,
        language,
        aspect_ratio,
        preview,
    } = options;
    let language = escape(language);
    let aspect_ratio = aspect_ratio.map_or_else(String::new, |ratio| {
        format!(" data-aspect-ratio=\"{}\"", escape(ratio))
    });
    let title = deck
        .groups
        .iter()
        .flat_map(|group| &group.slides)
        .find(|slide| !slide.title.trim().is_empty())
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
    let citations = if attribution.is_some() {
        " data-citations=\"true\""
    } else {
        ""
    };
    let attribution = attribution.map_or_else(String::new, |text| format!(
        "<div class=\"slides-attribution\" role=\"note\" aria-label=\"Citation processor attribution\">{} <a href=\"https://citationstyles.org/\">Citation Style Language</a> · <a href=\"licenses/citations-license.txt\">License</a></div>\n",
        escape(text)
    ));
    let styles = styles
        .iter()
        .map(|path| format!("<link rel=\"stylesheet\" href=\"{}\">\n", escape(path)))
        .collect::<String>();
    let preview_body = if preview {
        " data-preview=\"true\""
    } else {
        ""
    };
    let preview_script = if preview {
        "<pre class=\"slides-diagnostics\" aria-live=\"polite\" hidden></pre>\n<script src=\"assets/preview.js\"></script>\n"
    } else {
        ""
    };
    format!(
        "<!doctype html>\n<html lang=\"{language}\">\n<head>\n<meta charset=\"utf-8\">\n<meta http-equiv=\"Content-Security-Policy\" content=\"{}\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n<link rel=\"stylesheet\" href=\"assets/reset.css\">\n<link rel=\"stylesheet\" href=\"assets/reveal.css\">\n<link rel=\"stylesheet\" href=\"assets/theme.css\">\n<link rel=\"stylesheet\" href=\"assets/content.css\">\n{styles}</head>\n<body data-audience=\"{audience_name}\"{aspect_ratio}{citations}{preview_body}>\n{html}{attribution}<script src=\"assets/reveal.js\"></script>\n{notes}<script src=\"assets/bootstrap.js\"></script>\n{preview_script}</body>\n</html>\n",
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
    options: Options<'_>,
    remaining_resources: ProjectResourceLimits,
    cancellation: &dyn CancellationCheck,
) -> Result<GeneratedBundle, CliError> {
    let Options {
        audience,
        helper: slides_helper,
        data,
        preview,
    } = options;
    let analysis = &preprocessed.analysis;
    let language = analysis
        .attribute_environment()
        .resolve_at("lang", analysis.document().header().end)
        .and_then(|value| value.value.ok().flatten())
        .unwrap_or("");
    let deck = Deck::compile(analysis.document());
    let mut diagnostics = analysis.diagnostics().to_vec();
    let aspect_ratio = resolve_aspect_ratio(analysis, &mut diagnostics)?;
    let mut observations = Vec::new();
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
    });
    let mut body_selection = super::helper::Selection::default();
    let mut note_selection = super::helper::Selection::default();
    for formula in adocweave_core::output::projection::formulas(analysis) {
        if deck.contains_body_range(formula.source_range) {
            body_selection.equations.push(formula.source_range);
        } else if audience == Audience::Presenter && deck.contains_note_range(formula.source_range)
        {
            note_selection.equations.push(formula.source_range);
        }
    }
    for node in analysis
        .macros()
        .iter()
        .filter(|node| node.kind == semantic::StandardMacroKind::Footnote && visible(node.range))
    {
        if deck.contains_body_range(node.range) {
            body_selection.footnotes.push(node.range);
        } else {
            note_selection.footnotes.push(node.range);
        }
    }
    for citation in analysis
        .citations()
        .iter()
        .filter(|citation| visible(citation.range))
    {
        if deck.contains_body_range(citation.range) {
            body_selection.citations.push(citation.range);
        } else {
            note_selection.citations.push(citation.range);
        }
    }
    let selected = match super::helper::selected_content(
        analysis,
        &body_selection,
        &note_selection,
        audience == Audience::Presenter,
    ) {
        Ok(selected) => selected,
        Err(error) => {
            host_error(error, &mut diagnostics, &observations)?;
            return Ok(GeneratedBundle {
                files: Vec::new(),
                diagnostics,
                observations,
            });
        }
    };
    let external_csl =
        data.bibliography.is_some() || data.csl_style.is_some() || data.csl_locale.is_some();
    if external_csl {
        for entry in analysis.macros().iter().filter(|node| {
            node.kind == semantic::StandardMacroKind::BibliographyAnchor && visible(node.range)
        }) {
            problem(
                &mut diagnostics,
                "slides-bibliography-mode-conflict",
                "hand-written bibliography entries cannot be combined with CSL options; choose one bibliography mode",
                entry.range,
            );
        }
    }
    for (scope, content) in [
        (super::helper::Scope::Body, &selected.body),
        (super::helper::Scope::Notes, &selected.notes),
    ] {
        for citation in &content.citations {
            let manual = |key: &str| {
                analysis.macros().iter().any(|node| {
                    node.kind == semantic::StandardMacroKind::BibliographyAnchor
                        && node.target == key
                        && (deck.contains_body_range(node.range)
                            || (scope == super::helper::Scope::Notes
                                && deck.contains_note_range(node.range)))
                })
            };
            if !external_csl
                && citation.keys.iter().any(|key| {
                    !manual(&key.value)
                        && analysis.macros().iter().any(|node| {
                            node.kind == semantic::StandardMacroKind::BibliographyAnchor
                                && node.target == key.value
                                && deck.contains_note_range(node.range)
                        })
                })
            {
                problem(
                    &mut diagnostics,
                    "slides-note-only-reference",
                    "slide body cannot reference a bibliography entry defined only in presenter notes",
                    citation.range,
                );
            } else if !external_csl && citation.keys.iter().any(|key| !manual(&key.value)) {
                problem(
                    &mut diagnostics,
                    "slides-citation-data-required",
                    "unresolved citation key: supply --bibliography, --csl-style, and --csl-locale to use CSL citations",
                    citation.range,
                );
            }
        }
    }
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        return Ok(GeneratedBundle {
            files: Vec::new(),
            diagnostics,
            observations,
        });
    }
    let configured_roots = target.config.config.resource_roots();
    let default_data_roots = vec![authority.project_root().to_owned(), primary_base.to_owned()];
    let data_roots = if configured_roots.is_empty() {
        &default_data_roots
    } else {
        configured_roots
    };
    let styles = super::styles::load(
        &target.config.config,
        authority,
        data_roots,
        remaining_resources,
        cancellation,
    )?;
    let remaining_resources = styles.remaining;
    observations.extend(
        styles
            .resources
            .iter()
            .map(adocweave_project::ProjectBinaryResource::observation),
    );
    let data_inputs = super::data::load(
        data,
        !selected.body.equations.is_empty() || !selected.notes.equations.is_empty(),
        external_csl
            && (!selected.body.citations.is_empty() || !selected.notes.citations.is_empty()),
        authority,
        data_roots,
        remaining_resources,
        cancellation,
    )?;
    let remaining_resources = data_inputs.remaining;
    observations.extend(
        data_inputs
            .resources
            .iter()
            .map(adocweave_project::ProjectBinaryResource::observation),
    );
    if let Some(csl) = &data_inputs.csl {
        let keys = csl
            .items
            .iter()
            .filter_map(|item| item.get("id").and_then(serde_json::Value::as_str))
            .collect::<BTreeSet<_>>();
        for citation in selected
            .body
            .citations
            .iter()
            .chain(&selected.notes.citations)
        {
            if citation
                .keys
                .iter()
                .any(|key| !keys.contains(key.value.as_str()))
            {
                problem(
                    &mut diagnostics,
                    "slides-citation-unknown",
                    "citation key is absent from the supplied bibliography",
                    citation.range,
                );
            }
        }
    }
    let prepared =
        match super::helper::prepare(analysis, selected, data_inputs.macros, data_inputs.csl) {
            Ok(prepared) => prepared,
            Err(error) => {
                host_error(error, &mut diagnostics, &observations)?;
                return Ok(GeneratedBundle {
                    files: Vec::new(),
                    diagnostics,
                    observations,
                });
            }
        };
    for diagnostic in &prepared.diagnostics {
        if let Some(range) = diagnostic.range {
            problem(
                &mut diagnostics,
                &diagnostic.code,
                &diagnostic.message,
                range,
            );
        }
    }
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        return Ok(GeneratedBundle {
            files: Vec::new(),
            diagnostics,
            observations,
        });
    }
    let reserved_ids = deck.reserved_ids();
    let helper = match super::helper::execute_sync(
        &prepared,
        slides_helper,
        cancellation,
        super::helper::ProcessLimits::default(),
        &reserved_ids,
    ) {
        Ok(helper) => helper,
        Err(error) => {
            host_error(error, &mut diagnostics, &observations)?;
            return Ok(GeneratedBundle {
                files: Vec::new(),
                diagnostics,
                observations,
            });
        }
    };
    for diagnostic in &helper.diagnostics {
        let range = diagnostic
            .range
            .or_else(|| {
                diagnostic
                    .scope
                    .zip(diagnostic.key.as_ref())
                    .and_then(|(scope, key)| prepared.sources.get(&(scope, key.clone())).copied())
            })
            .unwrap_or_else(|| {
                adocweave_core::text::TextRange::new(
                    adocweave_core::text::TextSize::ZERO,
                    adocweave_core::text::TextSize::ZERO,
                )
                .expect("empty range")
            });
        problem(
            &mut diagnostics,
            &diagnostic.code,
            &diagnostic.message,
            range,
        );
        if diagnostic.severity == super::helper::protocol::Severity::Warning {
            diagnostics.last_mut().expect("added diagnostic").severity = Severity::Warning;
        }
    }
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == Severity::Error)
    {
        return Ok(GeneratedBundle {
            files: Vec::new(),
            diagnostics,
            observations,
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
            observations,
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
        .read_binary_resources(roots, &paths, remaining_resources, cancellation)
        .map_err(CliError::Project)?;
    observations.extend(
        acquired
            .iter()
            .map(adocweave_project::ProjectBinaryResource::observation),
    );
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
            observations,
        });
    }
    let mut policy = target.config.config.html_policy().clone();
    // The only resource resolutions here are our verified, digest-named files.
    policy.active_urls.allow_resolved_relative = true;
    let rendered = match deck.render(
        audience,
        &policy,
        &helper.inputs.body.clone().with_resources(body),
        &helper.inputs.notes.clone().with_resources(notes),
        OutputLimits::default(),
    ) {
        Ok(rendered) => rendered,
        Err(error) => {
            match &error {
                adocweave_core::output::html::HtmlRegionError::OutputLimit { limit, actual } => {
                    return Err(CliError::OutputLimit {
                        limit: *limit,
                        actual: *actual as u64,
                    });
                }
                adocweave_core::output::html::HtmlRegionError::GeneratedIdCollision {
                    range,
                    ..
                } => problem(
                    &mut diagnostics,
                    "slides-generated-id-collision",
                    &error.to_string(),
                    *range,
                ),
                adocweave_core::output::html::HtmlRegionError::FootnoteOutsideScope { range } => {
                    problem(
                        &mut diagnostics,
                        "slides-footnote-outside-scope",
                        &error.to_string(),
                        *range,
                    )
                }
                adocweave_core::output::html::HtmlRegionError::ReferenceOutsideScope { range } => {
                    problem(
                        &mut diagnostics,
                        "slides-reference-outside-scope",
                        &error.to_string(),
                        *range,
                    )
                }
                _ => return Err(CliError::Slides(error.to_string())),
            }
            return Ok(GeneratedBundle {
                files: Vec::new(),
                diagnostics,
                observations,
            });
        }
    };
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
    let mut files = fixed_files(audience, preview);
    files.extend(styles.files);
    helper_notices(&helper.notices, &mut files);
    files.extend(assets.into_values());
    let page = page(
        &deck,
        &rendered.html,
        PageOptions {
            audience,
            language,
            aspect_ratio: aspect_ratio.as_deref(),
            preview,
        },
        helper
            .notices
            .citations
            .as_ref()
            .map(|notices| notices.attribution.as_str()),
        &styles.links,
    );
    let limit = OutputLimits::default().max_output_bytes;
    if page.len() > limit as usize {
        return Err(CliError::OutputLimit {
            limit,
            actual: page.len() as u64,
        });
    }
    files.push(static_file(
        "index.html",
        BundleMediaType::Html,
        page.as_bytes(),
    ));
    adocweave_core::output::diagnostics::sort_diagnostics(&mut diagnostics);
    Ok(GeneratedBundle {
        files,
        diagnostics,
        observations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn citation_attribution_is_escaped_outside_the_author_slide_and_uses_fixed_links() {
        let analysis = adocweave_core::Engine::new(Default::default())
            .analyze("= Talk\n\n== Slide\n\nText.\n")
            .unwrap();
        let deck = Deck::compile(analysis.document());
        let html = page(
            &deck,
            "<div class=\"reveal\"><div class=\"slides\"></div></div>",
            PageOptions {
                audience: Audience::Public,
                language: "ja",
                aspect_ratio: None,
                preview: false,
            },
            Some("<script>Copyright & citation</script>"),
            &[],
        );
        assert!(html.contains("&lt;script&gt;Copyright &amp; citation&lt;/script&gt;"));
        assert!(html.contains("href=\"https://citationstyles.org/\""));
        assert!(html.contains("href=\"licenses/citations-license.txt\""));
        assert!(html.contains("role=\"note\" aria-label=\"Citation processor attribution\""));
        assert!(html.find("slides-attribution").unwrap() > html.find("</div></div>").unwrap());
        assert!(!html.contains("<script>Copyright"));
    }
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
            let html = page(
                &deck,
                &rendered.html,
                PageOptions {
                    audience,
                    language: "",
                    aspect_ratio: Some("16:9"),
                    preview: false,
                },
                None,
                &[],
            );
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
                fixed_files(audience, false)
                    .iter()
                    .any(|file| file.path == "assets/notes.js"),
                audience == Audience::Presenter
            );
        }
    }
}
