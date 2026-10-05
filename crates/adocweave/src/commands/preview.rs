use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use adocweave_core::output::diagnostics::Severity;
use adocweave_core::{CancellationCheck, CancellationToken};
use adocweave_project::{
    ProjectAuthority, ProjectConfigOverrides, ProjectConfigSelection, ProjectError, ProjectLimits,
    ProjectObservationAccess, ProjectObservationKind, ProjectRequest, ProjectResourceResult,
    ProjectSource, ProjectTarget, ProjectTargetResult, process,
};

use super::html_policy::{self, StylesheetArgument};
use crate::arguments::SlidesData;
use crate::cli_error::CliError;
use crate::preview;
use crate::slides::{Audience, bundle};

#[derive(Debug)]
pub(crate) enum Error {
    Input(String),
    Server(preview::Error),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ServerOptions {
    pub(crate) bind: IpAddr,
    pub(crate) port: u16,
    pub(crate) debounce_ms: u64,
}

pub(crate) struct RunRequest<'request> {
    pub(crate) project: ProjectRequest,
    pub(crate) watch: PreviewWatchAccess,
    pub(crate) css: &'request [StylesheetArgument],
    pub(crate) slides: Option<SlideOptions<'request>>,
    pub(crate) server: ServerOptions,
}

#[derive(Clone, Copy)]
pub(crate) struct SlideOptions<'a> {
    pub(crate) audience: Audience,
    pub(crate) helper: Option<&'a Path>,
    pub(crate) data: &'a SlidesData,
}

/// Retained filesystem access used only to detect changes between builds.
///
/// Source used for rendering is read exclusively by `adocweave-project`.
/// Polling retains opened roots so replacing a path in the ambient namespace
/// cannot redirect the preview to a different directory.
pub(crate) struct PreviewWatchAccess {
    access: ProjectObservationAccess,
}

impl PreviewWatchAccess {
    pub(crate) fn from_authority(authority: &ProjectAuthority) -> Self {
        Self {
            access: authority.observation_access(),
        }
    }

    fn snapshot(
        &self,
        dependencies: &[preview::Dependency],
    ) -> BTreeMap<preview::Dependency, preview::Fingerprint> {
        let mut observer = self.access.observer();
        dependencies
            .iter()
            .cloned()
            .map(|dependency| {
                let kind = match dependency.kind() {
                    preview::DependencyKind::Contents => ProjectObservationKind::Contents,
                    preview::DependencyKind::ContentsNoSymlinks => {
                        ProjectObservationKind::ContentsNoSymlinks
                    }
                    preview::DependencyKind::Existence => ProjectObservationKind::Existence,
                    preview::DependencyKind::BinaryContentsNoSymlinks => {
                        ProjectObservationKind::BinaryContentsNoSymlinks
                    }
                };
                let fingerprint = preview::Fingerprint::from_observation(
                    observer.observe(dependency.path(), kind),
                );
                (dependency, fingerprint)
            })
            .collect()
    }
}

struct PreviewProjectTemplate {
    targets: Vec<ProjectTarget>,
    sources: Vec<ProjectSource>,
    config: ProjectConfigSelection,
    overrides: ProjectConfigOverrides,
    apply_safe_fixes: bool,
    resource_selection: adocweave_project::ProjectResourceSelection,
    authority: ProjectAuthority,
    limits: ProjectLimits,
}

impl PreviewProjectTemplate {
    fn new(request: ProjectRequest) -> Result<Self, Error> {
        let ProjectRequest {
            targets,
            sources,
            config,
            overrides,
            apply_safe_fixes,
            resource_selection,
            authority,
            limits,
        } = request;
        let targets = match targets.as_slice() {
            [ProjectTarget::Path(path)] | [ProjectTarget::PathNoSymlinks(path)] => {
                vec![ProjectTarget::PathNoSymlinks(path.clone())]
            }
            _ => {
                return Err(Error::Input(
                    "preview requires exactly one AsciiDoc file".to_owned(),
                ));
            }
        };
        Ok(Self {
            targets,
            sources,
            config,
            overrides,
            apply_safe_fixes,
            resource_selection,
            authority,
            limits,
        })
    }

    fn request(&self) -> ProjectRequest {
        ProjectRequest {
            targets: self.targets.clone(),
            sources: self.sources.clone(),
            config: self.config.clone(),
            overrides: self.overrides.clone(),
            apply_safe_fixes: self.apply_safe_fixes,
            resource_selection: self.resource_selection,
            authority: self.authority.clone(),
            limits: self.limits,
        }
    }
}

pub(crate) fn run(request: RunRequest<'_>, shutdown: &AtomicBool) -> Result<(), Error> {
    if !request.server.bind.is_loopback() {
        eprintln!(
            "warning: preview is exposed on non-loopback address {}; rendered content may be visible to other hosts",
            request.server.bind
        );
    }
    let template = PreviewProjectTemplate::new(request.project)?;
    let snapshot_watch = request.watch;
    preview::run(
        preview::Options {
            bind: request.server.bind,
            port: request.server.port,
            debounce: Duration::from_millis(request.server.debounce_ms),
        },
        |cancellation| {
            build_with_slides(
                template.request(),
                request.css,
                request.slides,
                cancellation,
            )
        },
        move |dependencies| snapshot_watch.snapshot(dependencies),
        shutdown,
    )
    .map_err(Error::Server)
}

#[cfg(test)]
fn build(
    request: ProjectRequest,
    css: &[StylesheetArgument],
    cancellation: &CancellationToken,
) -> Result<preview::Build, String> {
    build_with_slides(request, css, None, cancellation)
}

fn failure(
    message: String,
    dependencies: BTreeMap<preview::Dependency, preview::Fingerprint>,
    slides: Option<SlideOptions<'_>>,
) -> preview::Build {
    match slides {
        Some(options) => preview::Build::slide_failure(message, dependencies, options.audience),
        None => preview::Build::failure(message, dependencies),
    }
}

fn build_with_slides(
    request: ProjectRequest,
    css: &[StylesheetArgument],
    slides: Option<SlideOptions<'_>>,
    cancellation: &CancellationToken,
) -> Result<preview::Build, String> {
    let authority = request.authority.clone();
    let limits = request.limits;
    let result = match process(request, cancellation) {
        Ok(result) => result,
        Err(ProjectError::Cancelled) => return Err(ProjectError::Cancelled.to_string()),
        Err(error) => {
            let dependencies = error
                .repair_candidate()
                .map_or_else(BTreeMap::new, |candidate| {
                    let dependency = dependency(candidate);
                    BTreeMap::from([(
                        dependency,
                        preview::Fingerprint::from_observation(candidate.observation.clone()),
                    )])
                });
            return Ok(failure(error.to_string(), dependencies, slides));
        }
    };
    let mut dependencies = dependencies(&result.resources);
    let Some(target) = result.targets.first() else {
        return Ok(failure(
            "project processing returned no preview target".to_owned(),
            dependencies,
            slides,
        ));
    };
    merge_dependencies(&mut dependencies, &target.resources);
    if let Some(options) = slides {
        return build_slides(
            target,
            options,
            &authority,
            limits,
            &result.usage,
            dependencies,
            cancellation,
        );
    }
    match build_target(target, css, dependencies) {
        Ok(build) => Ok(build),
        Err(BuildError::Message(message, dependencies)) => {
            Ok(preview::Build::failure(message, dependencies))
        }
    }
}

fn build_slides(
    target: &ProjectTargetResult,
    options: SlideOptions<'_>,
    authority: &ProjectAuthority,
    limits: ProjectLimits,
    usage: &adocweave_project::ProjectUsage,
    mut dependencies: BTreeMap<preview::Dependency, preview::Fingerprint>,
    cancellation: &CancellationToken,
) -> Result<preview::Build, String> {
    let slides = Some(options);
    let analysis = match target
        .analysis
        .as_ref()
        .map_err(|error| error.to_string())
        .and_then(|analysis| {
            analysis
                .expanded
                .as_ref()
                .map_err(|error| error.to_string())
        }) {
        Ok(analysis) => analysis,
        Err(message) => return Ok(failure(message, dependencies, slides)),
    };
    let configured = target.config.config.resource_limits();
    let remaining = adocweave_project::ProjectResourceLimits {
        max_files: configured
            .max_files
            .min(limits.resources.max_files)
            .saturating_sub(usage.read_operations as usize),
        max_resource_bytes: configured
            .max_resource_bytes
            .min(limits.resources.max_resource_bytes),
        max_total_bytes: configured
            .max_total_bytes
            .min(limits.resources.max_total_bytes)
            .saturating_sub(usage.read_bytes),
    };
    let primary_base = target
        .path
        .as_deref()
        .and_then(Path::parent)
        .unwrap_or_else(|| authority.project_root());
    let built = match bundle::build(
        &analysis.preprocessed,
        target,
        authority,
        primary_base,
        bundle::Options::convert(options.audience, options.helper, options.data).with_preview(),
        remaining,
        cancellation,
    ) {
        Ok(built) => built,
        Err(error) => {
            if cancellation.is_cancelled() {
                return Err(error.to_string());
            }
            match &error {
                CliError::SlidesResources { observations, .. } => {
                    merge_observations(&mut dependencies, observations)
                }
                CliError::Project(error) => {
                    if let Some(candidate) = error.repair_candidate() {
                        merge_observations(&mut dependencies, std::slice::from_ref(candidate));
                    }
                }
                _ => {}
            }
            return Ok(failure(error.to_string(), dependencies, slides));
        }
    };
    merge_observations(&mut dependencies, &built.observations);
    let diagnostics = preview_diagnostics(target, &built.diagnostics, authority.project_root())?;
    if diagnostics_have_errors(target, &built.diagnostics) {
        let mut failed = failure(
            "slide generation reported errors".to_owned(),
            dependencies,
            slides,
        );
        failed.diagnostics = preview::serialize_diagnostics(&diagnostics);
        return Ok(failed);
    }
    let snapshot =
        match adocweave_project::BundleSnapshot::from_files(built.files, limits, cancellation) {
            Ok(snapshot) => snapshot,
            Err(error) if cancellation.is_cancelled() => return Err(error.to_string()),
            Err(error) => return Ok(failure(error.to_string(), dependencies, slides)),
        };
    let audience = match bundle::audience_from_bundle(&snapshot) {
        Ok(audience) => audience,
        Err(error) => return Ok(failure(error.to_string(), dependencies, slides)),
    };
    Ok(preview::Build::slides(
        snapshot,
        audience,
        preview::serialize_diagnostics(&diagnostics),
        dependencies,
    ))
}

fn merge_observations(
    dependencies: &mut BTreeMap<preview::Dependency, preview::Fingerprint>,
    observations: &[adocweave_project::ProjectObservationCandidate],
) {
    for candidate in observations {
        dependencies.insert(
            dependency(candidate),
            preview::Fingerprint::from_observation(candidate.observation.clone()),
        );
    }
}

fn diagnostics_have_errors(
    target: &ProjectTargetResult,
    diagnostics: &[adocweave_core::output::diagnostics::Diagnostic],
) -> bool {
    diagnostics
        .iter()
        .any(|item| item.severity == Severity::Error)
        || target
            .analysis
            .as_ref()
            .ok()
            .and_then(|analysis| analysis.expanded.as_ref().ok())
            .is_some_and(|analysis| {
                analysis
                    .source_mapping
                    .diagnostics
                    .iter()
                    .any(|item| item.diagnostic.severity == Severity::Error)
                    || analysis
                        .local_target_diagnostics
                        .iter()
                        .any(|item| item.diagnostic.severity == Severity::Error)
            })
}

fn preview_diagnostics(
    target: &ProjectTargetResult,
    output: &[adocweave_core::output::diagnostics::Diagnostic],
    current: &Path,
) -> Result<Vec<preview::PreviewDiagnostic>, String> {
    let analysis = target
        .analysis
        .as_ref()
        .map_err(|error| error.to_string())?
        .expanded
        .as_ref()
        .map_err(|error| error.to_string())?;
    let sources = crate::project_command::diagnostic_sources(target, current)
        .map_err(|error| error.to_string())?;
    analysis
        .source_mapping
        .diagnostics
        .iter()
        .map(|item| &item.diagnostic)
        .chain(
            analysis
                .local_target_diagnostics
                .iter()
                .map(|item| &item.diagnostic),
        )
        .chain(output)
        .map(|diagnostic| {
            let report = super::convert::render_diagnostics(
                &analysis.preprocessed,
                std::slice::from_ref(diagnostic),
                &sources,
            )
            .map_err(|error| crate::cli_error::convert_error(error).to_string())?;
            let mut diagnostic = diagnostic.clone();
            diagnostic.message = report.output.trim_end().to_owned();
            Ok(preview::PreviewDiagnostic::Analysis(diagnostic))
        })
        .collect()
}

enum BuildError {
    Message(String, BTreeMap<preview::Dependency, preview::Fingerprint>),
}

fn build_target(
    target: &ProjectTargetResult,
    css: &[StylesheetArgument],
    dependencies: BTreeMap<preview::Dependency, preview::Fingerprint>,
) -> Result<preview::Build, BuildError> {
    let analysis = target
        .analysis
        .as_ref()
        .map_err(|error| BuildError::Message(error.to_string(), dependencies.clone()))?
        .expanded
        .as_ref()
        .map_err(|error| BuildError::Message(error.to_string(), dependencies.clone()))?;
    let policy = html_policy::build_project(&target.config.config, &target.resources, true, css)
        .map_err(|error| BuildError::Message(error.to_string(), dependencies.clone()))?;
    let output = html_policy::render_checked(analysis.preprocessed.analysis.document(), &policy)
        .map_err(|error| BuildError::Message(error.to_string(), dependencies.clone()))?;
    let mut diagnostics = analysis
        .source_mapping
        .diagnostics
        .iter()
        .map(|item| preview::PreviewDiagnostic::Analysis(item.diagnostic.clone()))
        .collect::<Vec<_>>();
    diagnostics.extend(analysis.local_target_diagnostics.iter().map(|item| {
        if item.diagnostic.code.as_str().starts_with("local-include-") {
            preview::PreviewDiagnostic::include(
                item.diagnostic.code.as_str(),
                item.diagnostic.message.clone(),
                &item.target,
            )
        } else {
            preview::PreviewDiagnostic::Analysis(item.diagnostic.clone())
        }
    }));
    diagnostics.extend(preview::PreviewDiagnostic::analysis(&output.diagnostics));
    Ok(preview::Build::new(
        output.html,
        preview::serialize_diagnostics(&diagnostics),
        dependencies,
    )
    .with_style_origins(html_policy::external_origins(&policy)))
}

fn dependencies(
    resources: &[ProjectResourceResult],
) -> BTreeMap<preview::Dependency, preview::Fingerprint> {
    let mut dependencies = BTreeMap::new();
    merge_dependencies(&mut dependencies, resources);
    dependencies
}

fn merge_dependencies(
    dependencies: &mut BTreeMap<preview::Dependency, preview::Fingerprint>,
    resources: &[ProjectResourceResult],
) {
    for resource in resources {
        let Some(observation) = resource.observation.as_ref() else {
            continue;
        };
        let dependency = dependency(observation);
        let fingerprint = preview::Fingerprint::from_observation(observation.observation.clone());
        dependencies.insert(dependency, fingerprint);
    }
}

fn dependency(candidate: &adocweave_project::ProjectObservationCandidate) -> preview::Dependency {
    match candidate.kind {
        ProjectObservationKind::Contents => preview::Dependency::contents(candidate.path.clone()),
        ProjectObservationKind::ContentsNoSymlinks => {
            preview::Dependency::contents_no_symlinks(candidate.path.clone())
        }
        ProjectObservationKind::Existence => preview::Dependency::existence(candidate.path.clone()),
        ProjectObservationKind::BinaryContentsNoSymlinks => {
            preview::Dependency::binary_contents_no_symlinks(candidate.path.clone())
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Input(message) => formatter.write_str(message),
            Self::Server(source) => source.fmt(formatter),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use adocweave_project::{
        ProjectAuthority, ProjectConfigOverrides, ProjectConfigSelection, ProjectLimits,
        ProjectResourceSelection, ProjectTarget,
    };

    use super::*;

    fn slides_request(root: &Path) -> ProjectRequest {
        ProjectRequest {
            targets: vec![ProjectTarget::Path(PathBuf::from("talk.adoc"))],
            sources: Vec::new(),
            config: ProjectConfigSelection::Discover,
            overrides: ProjectConfigOverrides::default(),
            apply_safe_fixes: false,
            resource_selection: ProjectResourceSelection {
                local_targets: false,
                stylesheets: false,
            },
            authority: ProjectAuthority::open(root, [root.to_owned()]).unwrap(),
            limits: ProjectLimits::default(),
        }
    }

    fn public_slide_build(root: &Path, data: &SlidesData) -> preview::Build {
        let helper =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/slides-helper/bin.mjs");
        build_with_slides(
            slides_request(root),
            &[],
            Some(SlideOptions {
                audience: Audience::Public,
                helper: Some(&helper),
                data,
            }),
            &CancellationToken::new(),
        )
        .unwrap()
    }

    #[test]
    fn initial_macro_json_and_protocol_errors_are_repairable_and_note_only_files_are_unwatched() {
        let root = tempfile::tempdir().unwrap();
        let document = root.path().join("talk.adoc");
        let macros = root.path().join("macros.json");
        let visible = "= Talk\n\n== Body\n\nlatexmath:[\\R]\n";
        let data = SlidesData {
            math_macros: Some(PathBuf::from("macros.json")),
            ..Default::default()
        };
        for (invalid, expected) in [
            ("[{", "invalid math macro JSON"),
            (
                r#"[{"name":"R","definition":"\\mathbb{R}","arguments":99}]"#,
                "helper-protocol",
            ),
        ] {
            std::fs::write(&document, visible).unwrap();
            std::fs::write(&macros, invalid).unwrap();
            let failed = public_slide_build(root.path(), &data);
            assert!(failed.html().contains("Preview error"), "{}", failed.html());
            assert!(failed.has_dependency(&macros));
            assert!(
                failed.diagnostics.contains(expected),
                "{}",
                failed.diagnostics
            );

            std::fs::write(&macros, r#"[{"name":"R","definition":"\\mathbb{R}"}]"#).unwrap();
            let repaired = public_slide_build(root.path(), &data);
            assert!(repaired.has_dependency(&macros));
            assert!(
                repaired.html().contains("math-rendered"),
                "{}",
                repaired.html()
            );
            assert!(!repaired.html().contains("Preview error"));

            std::fs::write(
                &document,
                "= Talk\n\n== Body\n\nPublic.\n\n[.notes]\n--\nlatexmath:[\\R]\n--\n",
            )
            .unwrap();
            std::fs::write(&macros, invalid).unwrap();
            let note_only = public_slide_build(root.path(), &data);
            assert!(note_only.html().contains("Public."));
            assert!(!note_only.html().contains("Preview error"));
            assert!(!note_only.has_dependency(&macros));
            assert!(!note_only.html().contains("math-rendered"));
        }
    }

    #[test]
    fn initial_csl_json_and_xml_errors_are_repairable_and_note_only_files_are_unwatched() {
        let root = tempfile::tempdir().unwrap();
        let document = root.path().join("talk.adoc");
        let resources = [
            ("references.json", r#"[{"id":"result","title":"Result"}]"#),
            (
                "style.csl",
                include_str!("../../../../packages/slides-helper/fixtures/numeric.csl"),
            ),
            (
                "locale.xml",
                include_str!("../../../../packages/slides-helper/fixtures/locale-en-US.xml"),
            ),
        ];
        let data = SlidesData {
            bibliography: Some(PathBuf::from("references.json")),
            csl_style: Some(PathBuf::from("style.csl")),
            csl_locale: Some(PathBuf::from("locale.xml")),
            ..Default::default()
        };
        for (invalid_file, expected) in [
            ("references.json", "invalid bibliography JSON"),
            ("style.csl", "invalid style XML"),
            ("locale.xml", "invalid locale XML"),
        ] {
            std::fs::write(&document, "= Talk\n\n== Body\n\ncite:[result].\n").unwrap();
            for (file, valid) in resources {
                std::fs::write(root.path().join(file), valid).unwrap();
            }
            let invalid_path = root.path().join(invalid_file);
            std::fs::write(&invalid_path, "malformed [<").unwrap();
            let failed = public_slide_build(root.path(), &data);
            assert!(failed.html().contains("Preview error"), "{}", failed.html());
            assert!(failed.has_dependency(&invalid_path));
            assert!(
                failed.diagnostics.contains(expected),
                "{}",
                failed.diagnostics
            );

            let valid = resources
                .iter()
                .find(|(file, _)| *file == invalid_file)
                .unwrap()
                .1;
            std::fs::write(&invalid_path, valid).unwrap();
            let repaired = public_slide_build(root.path(), &data);
            assert!(
                !repaired.html().contains("Preview error"),
                "{}",
                repaired.html()
            );
            assert!(repaired.html().contains("slides-body-references"));
            for (file, _) in resources {
                assert!(repaired.has_dependency(&root.path().join(file)), "{file}");
            }

            std::fs::write(
                &document,
                "= Talk\n\n== Body\n\nPublic.\n\n[.notes]\n--\ncite:[result].\n--\n",
            )
            .unwrap();
            for (file, _) in resources {
                std::fs::write(root.path().join(file), "malformed [<").unwrap();
            }
            let note_only = public_slide_build(root.path(), &data);
            assert!(
                !note_only.html().contains("Preview error"),
                "{}",
                note_only.html()
            );
            assert!(note_only.html().contains("Public."));
            assert!(!note_only.html().contains("slides-body-references"));
            for (file, _) in resources {
                assert!(!note_only.has_dependency(&root.path().join(file)), "{file}");
            }
        }
    }

    #[test]
    fn public_preview_never_opens_or_watches_note_only_images_and_data_flags() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("talk.adoc"), "= Public\n\n== Visible\n\nBody.\n\n[.notes]\n--\nstem:[x+y] cite:[private]\n\nimage::private.svg[]\n--\n").unwrap();
        let data = SlidesData {
            bibliography: Some(PathBuf::from("missing.json")),
            csl_style: Some(PathBuf::from("missing.csl")),
            csl_locale: Some(PathBuf::from("missing.xml")),
            math_macros: Some(PathBuf::from("missing-macros.json")),
        };
        let build = build_with_slides(
            slides_request(root.path()),
            &[],
            Some(SlideOptions {
                audience: Audience::Public,
                helper: Some(Path::new("missing-helper")),
                data: &data,
            }),
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(build.html().contains("Body."), "{}", build.html());
        assert!(!build.html().contains("private"));
        assert!(build.html().contains("assets/preview.js"));
        for path in [
            "private.svg",
            "missing.json",
            "missing.csl",
            "missing.xml",
            "missing-macros.json",
        ] {
            assert!(!build.has_dependency(&root.path().join(path)), "{path}");
        }
    }

    #[test]
    fn initial_missing_visible_image_is_watched_and_can_be_repaired() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("talk.adoc"),
            "= Talk\n\n== Figure\n\nimage::figure.svg[]\n",
        )
        .unwrap();
        let data = SlidesData::default();
        let options = Some(SlideOptions {
            audience: Audience::Presenter,
            helper: None,
            data: &data,
        });
        let failed = build_with_slides(
            slides_request(root.path()),
            &[],
            options,
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(failed.html().contains("Preview error"));
        assert!(failed.has_dependency(&root.path().join("figure.svg")));
        std::fs::write(root.path().join("figure.svg"), "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"20\"><rect width=\"20\" height=\"20\" fill=\"blue\"/></svg>").unwrap();
        let repaired = build_with_slides(
            slides_request(root.path()),
            &[],
            options,
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(repaired.html().contains(".svg\""));
        assert!(!repaired.html().contains("Preview error"));
    }

    #[test]
    fn local_css_errors_are_repairable_and_edits_update_the_content_addressed_link() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("talk.adoc"),
            "= Talk\n\n== Body\n\nText.\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join(".adocweave.toml"),
            "schema-version = 2\n[html]\nstylesheet-files = [\"theme.css\"]\n",
        )
        .unwrap();
        let css = root.path().join("theme.css");
        std::fs::write(&css, [255]).unwrap();
        let data = SlidesData::default();
        let options = Some(SlideOptions {
            audience: Audience::Presenter,
            helper: None,
            data: &data,
        });
        let failed = build_with_slides(
            slides_request(root.path()),
            &[],
            options,
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(failed.has_dependency(&css));
        assert!(failed.html().contains("must be UTF-8"));
        std::fs::write(&css, ".reveal { color: red; }").unwrap();
        let red = build_with_slides(
            slides_request(root.path()),
            &[],
            options,
            &CancellationToken::new(),
        )
        .unwrap();
        std::fs::write(&css, ".reveal { color: blue; }").unwrap();
        let blue = build_with_slides(
            slides_request(root.path()),
            &[],
            options,
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(red.has_dependency(&css) && blue.has_dependency(&css));
        assert_ne!(red.html(), blue.html());
        assert!(!blue.html().contains("Preview error"));
    }

    #[test]
    fn slide_render_errors_in_included_source_keep_the_original_location() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join(".adocweave.toml"),
            "schema-version = 2\n[resources]\ninclude = true\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("talk.adoc"),
            "= Talk\n\n== Body\n\ninclude::part.adoc[]\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("part.adoc"),
            "[%step]\nUnsupported stepped paragraph.\n",
        )
        .unwrap();
        let data = SlidesData::default();
        let build = build_with_slides(
            slides_request(root.path()),
            &[],
            Some(SlideOptions {
                audience: Audience::Presenter,
                helper: None,
                data: &data,
            }),
            &CancellationToken::new(),
        )
        .unwrap();
        assert!(
            build.diagnostics.contains("include:part.adoc:"),
            "{}",
            build.diagnostics
        );
        assert!(
            build.diagnostics.contains("slides-invalid-step"),
            "{}",
            build.diagnostics
        );
    }

    #[test]
    fn one_build_uses_one_project_request_for_all_resources() {
        let root = tempfile::tempdir().expect("project root");
        std::fs::write(
            root.path().join(".adocweave.toml"),
            "schema-version = 2\n[resources]\ninclude = true\n[local-targets]\nenabled = true\nproject-root = \".\"\n[html]\nstylesheet-files = [\"configured.css\"]\n",
        )
        .expect("configuration");
        std::fs::write(
            root.path().join("manual.adoc"),
            "include::part.adoc[]\nxref:target.adoc[target]\n",
        )
        .expect("document");
        std::fs::write(root.path().join("part.adoc"), "included\n").expect("include");
        std::fs::write(root.path().join("configured.css"), "body{}\n").expect("stylesheet");
        std::fs::write(root.path().join("target.adoc"), "target\n").expect("local target");
        let authority = ProjectAuthority::open(root.path(), [root.path().to_owned()])
            .expect("project authority");
        let request = ProjectRequest {
            targets: vec![ProjectTarget::Path(PathBuf::from("manual.adoc"))],
            sources: Vec::new(),
            config: ProjectConfigSelection::Discover,
            overrides: ProjectConfigOverrides::default(),
            apply_safe_fixes: false,
            resource_selection: ProjectResourceSelection {
                local_targets: true,
                stylesheets: true,
            },
            authority,
            limits: ProjectLimits::default(),
        };
        let build = build(request, &[], &CancellationToken::new()).expect("preview build");
        assert!(build.html().contains("included"), "{}", build.html());
        assert!(build.html().contains("body{}"), "{}", build.html());
        assert_eq!(build.dependency_count(), 6);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn project_and_watcher_keep_their_opened_root_after_namespace_replacement() {
        let parent = tempfile::tempdir().expect("temporary parent");
        let root = parent.path().join("workspace");
        std::fs::create_dir(&root).expect("workspace");
        let config = root.join(".adocweave.toml");
        let document = root.join("manual.adoc");
        let include = root.join("part.adoc");
        let stylesheet = root.join("theme.css");
        std::fs::write(
            &config,
            "schema-version = 2\n[resources]\ninclude = true\n[html]\nstylesheet-files = [\"theme.css\"]\n",
        )
        .expect("configuration");
        std::fs::write(&document, "include::part.adoc[]\n").expect("document");
        std::fs::write(&include, "TRUSTED_INCLUDE\n").expect("include");
        std::fs::write(&stylesheet, "/* TRUSTED_STYLE */\n").expect("stylesheet");
        let authority = ProjectAuthority::open(&root, [root.clone()]).expect("project authority");
        let watch = PreviewWatchAccess::from_authority(&authority);
        let request = ProjectRequest {
            targets: vec![ProjectTarget::Path(PathBuf::from("manual.adoc"))],
            sources: Vec::new(),
            config: ProjectConfigSelection::Discover,
            overrides: ProjectConfigOverrides::default(),
            apply_safe_fixes: false,
            resource_selection: ProjectResourceSelection {
                local_targets: true,
                stylesheets: true,
            },
            authority,
            limits: ProjectLimits::default(),
        };
        let displaced = parent.path().join("opened-workspace");
        std::fs::rename(&root, &displaced).expect("displace workspace");
        std::fs::create_dir(&root).expect("replacement workspace");
        std::fs::write(&config, "schema-version = 2\n").expect("replacement configuration");
        std::fs::write(&document, "OUTSIDE_DOCUMENT\n").expect("replacement document");
        std::fs::write(&include, "OUTSIDE_INCLUDE\n").expect("replacement include");
        std::fs::write(&stylesheet, "/* OUTSIDE_STYLE */\n").expect("replacement stylesheet");

        let snapshots = watch.snapshot(&[
            preview::Dependency::contents_no_symlinks(document.clone()),
            preview::Dependency::contents_no_symlinks(include.clone()),
            preview::Dependency::contents_no_symlinks(stylesheet.clone()),
        ]);
        assert_eq!(
            snapshots.get(&preview::Dependency::contents_no_symlinks(document.clone())),
            Some(&preview::Fingerprint::from_loaded_bytes(
                b"include::part.adoc[]\n"
            ))
        );
        assert_eq!(
            snapshots.get(&preview::Dependency::contents_no_symlinks(include.clone())),
            Some(&preview::Fingerprint::from_loaded_bytes(
                b"TRUSTED_INCLUDE\n"
            ))
        );
        assert_eq!(
            snapshots.get(&preview::Dependency::contents_no_symlinks(
                stylesheet.clone()
            )),
            Some(&preview::Fingerprint::from_loaded_bytes(
                b"/* TRUSTED_STYLE */\n"
            ))
        );
        let build = build(request, &[], &CancellationToken::new()).expect("preview build");
        assert!(build.html().contains("TRUSTED_INCLUDE"), "{}", build.html());
        assert!(build.html().contains("TRUSTED_STYLE"), "{}", build.html());
        assert!(!build.html().contains("OUTSIDE"), "{}", build.html());

        std::fs::remove_dir_all(&root).expect("remove replacement workspace");
        std::fs::rename(displaced, &root).expect("restore workspace");
    }

    #[test]
    fn invalid_configuration_failure_retains_its_observation() {
        let root = tempfile::tempdir().expect("project root");
        let config = root.path().join(".adocweave.toml");
        std::fs::write(&config, "not valid TOML = [\n").expect("configuration");
        std::fs::write(root.path().join("manual.adoc"), "text\n").expect("document");
        let authority = ProjectAuthority::open(root.path(), [root.path().to_owned()])
            .expect("project authority");
        let request = ProjectRequest {
            targets: vec![ProjectTarget::Path(PathBuf::from("manual.adoc"))],
            sources: Vec::new(),
            config: ProjectConfigSelection::Discover,
            overrides: ProjectConfigOverrides::default(),
            apply_safe_fixes: false,
            resource_selection: ProjectResourceSelection::default(),
            authority,
            limits: ProjectLimits::default(),
        };
        let build =
            build(request, &[], &CancellationToken::new()).expect("recoverable preview failure");
        assert!(build.html().contains("Preview error"));
        assert!(build.has_dependency(&config));
    }

    #[test]
    fn cancelled_project_request_is_not_turned_into_an_error_page() {
        let root = tempfile::tempdir().expect("project root");
        std::fs::write(root.path().join("manual.adoc"), "text\n").expect("document");
        let authority = ProjectAuthority::open(root.path(), [root.path().to_owned()])
            .expect("project authority");
        let request = ProjectRequest {
            targets: vec![ProjectTarget::Path(PathBuf::from("manual.adoc"))],
            sources: Vec::new(),
            config: ProjectConfigSelection::Disabled,
            overrides: ProjectConfigOverrides::default(),
            apply_safe_fixes: false,
            resource_selection: ProjectResourceSelection::default(),
            authority,
            limits: ProjectLimits::default(),
        };
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(build(request, &[], &cancellation).is_err());
    }

    #[test]
    fn preview_template_rejects_directory_selection_before_starting_the_server() {
        let root = tempfile::tempdir().expect("project root");
        let authority = ProjectAuthority::open(root.path(), [root.path().to_owned()])
            .expect("project authority");
        let request = ProjectRequest {
            targets: vec![ProjectTarget::Directory(PathBuf::from("."))],
            sources: Vec::new(),
            config: ProjectConfigSelection::Disabled,
            overrides: ProjectConfigOverrides::default(),
            apply_safe_fixes: false,
            resource_selection: ProjectResourceSelection::default(),
            authority,
            limits: ProjectLimits::default(),
        };

        assert!(matches!(
            PreviewProjectTemplate::new(request),
            Err(Error::Input(message)) if message == "preview requires exactly one AsciiDoc file"
        ));
    }
}
