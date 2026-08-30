use std::collections::BTreeMap;
use std::net::IpAddr;
#[cfg(test)]
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use adocweave::CancellationToken;
use adocweave_project::{
    ProjectAuthority, ProjectConfigOverrides, ProjectConfigSelection, ProjectError, ProjectLimits,
    ProjectObservationAccess, ProjectObservationKind, ProjectRequest, ProjectResourceResult,
    ProjectSource, ProjectTarget, ProjectTargetResult, process,
};

use super::html_policy::{self, StylesheetArgument};
use crate::preview;

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
    pub(crate) server: ServerOptions,
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
        |cancellation| build(template.request(), request.css, cancellation),
        move |dependencies| snapshot_watch.snapshot(dependencies),
        shutdown,
    )
    .map_err(Error::Server)
}

fn build(
    request: ProjectRequest,
    css: &[StylesheetArgument],
    cancellation: &CancellationToken,
) -> Result<preview::Build, String> {
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
            return Ok(preview::Build::failure(error.to_string(), dependencies));
        }
    };
    let mut dependencies = dependencies(&result.resources);
    let Some(target) = result.targets.first() else {
        return Ok(preview::Build::failure(
            "project processing returned no preview target".to_owned(),
            dependencies,
        ));
    };
    merge_dependencies(&mut dependencies, &target.resources);
    match build_target(target, css, dependencies) {
        Ok(build) => Ok(build),
        Err(BuildError::Message(message, dependencies)) => {
            Ok(preview::Build::failure(message, dependencies))
        }
    }
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
        assert!(build.html.contains("included"), "{}", build.html);
        assert!(build.html.contains("body{}"), "{}", build.html);
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
        assert!(build.html.contains("TRUSTED_INCLUDE"), "{}", build.html);
        assert!(build.html.contains("TRUSTED_STYLE"), "{}", build.html);
        assert!(!build.html.contains("OUTSIDE"), "{}", build.html);

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
        assert!(build.html.contains("Preview error"));
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
