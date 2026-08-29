use std::fs;
use std::path::{Path, PathBuf};

use adocweave::OutputLimits;
use adocweave_host::{FilesystemReadLimits, LocalFilesystemPolicy, ResourceError};
use adocweave_project::{
    ConfigSelection, ProjectLimit, ProjectLimits, ProjectOverrides, ProjectRequest,
    ProjectResourceFailure, ProjectResourceKind, ProjectResourceOutcome, ProjectTarget,
    ProjectTargetError, process,
};

fn request(root: &Path, targets: Vec<ProjectTarget>) -> ProjectRequest {
    let filesystem_reads = FilesystemReadLimits::default();
    ProjectRequest {
        project_root: root.to_owned(),
        targets,
        config: ConfigSelection::Discover,
        overrides: ProjectOverrides::default(),
        authority: LocalFilesystemPolicy::new([root.to_owned()], filesystem_reads)
            .expect("temporary project is valid authority"),
        limits: ProjectLimits {
            filesystem_reads,
            max_directory_entries: 10_000,
            max_processing_iterations: 100,
            output: OutputLimits::default(),
        },
    }
}

fn write(path: impl AsRef<Path>, source: &str) {
    fs::write(path, source).expect("fixture is written");
}

#[test]
fn processes_discovered_config_include_stylesheet_and_local_target() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    fs::create_dir(root.join("styles")).expect("stylesheet directory");
    write(
        root.join(".adocweave.toml"),
        r#"schema-version = 2
[resources]
include = true
roots = ["."]
[local-targets]
enabled = true
project-root = "."
[html]
stylesheet-files = ["styles/manual.css"]
"#,
    );
    write(
        root.join("guide.adoc"),
        "= Guide\n\ninclude::part.adoc[]\n\nimage::asset.txt[]\n",
    );
    write(
        root.join("part.adoc"),
        "Included text.\n\nimage::asset.txt[]\n",
    );
    write(root.join("styles/manual.css"), "body {}\n");
    write(root.join("asset.txt"), "asset\n");

    let result = process(request(
        root,
        vec![ProjectTarget::Glob("*.adoc".to_owned())],
    ))
    .expect("project processing succeeds");
    let guide = result
        .targets
        .iter()
        .find(|target| target.path.ends_with("guide.adoc"))
        .expect("guide is selected");
    let analysis = guide.outcome.as_ref().expect("guide is analyzed");
    assert!(analysis.document.source.contains("Included text."));
    for kind in [
        ProjectResourceKind::Config,
        ProjectResourceKind::Primary,
        ProjectResourceKind::Include,
        ProjectResourceKind::Stylesheet,
        ProjectResourceKind::LocalTarget,
    ] {
        assert!(guide.resources.iter().any(|resource| resource.kind == kind));
    }
    assert!(guide.resources.iter().any(|resource| {
        resource.kind == ProjectResourceKind::LocalTarget
            && resource.outcome == ProjectResourceOutcome::Present
    }));
    assert!(result.targets.iter().all(|target| target.outcome.is_ok()));
}

#[test]
fn missing_primary_is_confined_to_one_target() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let result = process(request(
        directory.path(),
        vec![ProjectTarget::Path(PathBuf::from("missing.adoc"))],
    ))
    .expect("request remains coherent");
    assert!(matches!(
        result.targets[0].outcome,
        Err(ProjectTargetError::Read(ResourceError::Missing(_)))
    ));
}

#[test]
fn processing_iteration_limit_returns_incomplete_target() {
    let directory = tempfile::tempdir().expect("temporary directory");
    write(
        directory.path().join("guide.adoc"),
        "include::part.adoc[]\n",
    );
    write(directory.path().join("part.adoc"), "included\n");
    let mut request = request(
        directory.path(),
        vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
    );
    request.config = ConfigSelection::Disabled;
    request.overrides.include = Some(true);
    request.limits.max_processing_iterations = 1;

    let result = process(request).expect("request remains coherent");
    assert!(matches!(
        result.targets[0].outcome,
        Err(ProjectTargetError::Incomplete(
            ProjectLimit::ProcessingIterations { limit: 1 }
        ))
    ));
}

#[cfg(unix)]
#[test]
fn symlinked_include_is_rejected() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary directory");
    let outside = tempfile::tempdir().expect("outside directory");
    write(outside.path().join("secret.adoc"), "secret\n");
    symlink(
        outside.path().join("secret.adoc"),
        directory.path().join("linked.adoc"),
    )
    .expect("symlink fixture");
    write(
        directory.path().join("guide.adoc"),
        "include::linked.adoc[]\n",
    );
    let mut request = request(
        directory.path(),
        vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
    );
    request.config = ConfigSelection::Disabled;
    request.overrides.include = Some(true);

    let result = process(request).expect("request remains coherent");
    let target = &result.targets[0];
    assert!(target.outcome.is_err());
    assert!(target.resources.iter().any(|resource| {
        resource.kind == ProjectResourceKind::Include
            && matches!(
                resource.outcome,
                ProjectResourceOutcome::Failed(ProjectResourceFailure::Rejected(_))
            )
    }));
}

#[test]
fn request_read_budget_is_shared_between_targets() {
    let directory = tempfile::tempdir().expect("temporary directory");
    write(directory.path().join("a.adoc"), "a\n");
    write(directory.path().join("b.adoc"), "b\n");
    let mut request = request(
        directory.path(),
        vec![ProjectTarget::Directory(PathBuf::from("."))],
    );
    request.config = ConfigSelection::Disabled;
    request.limits.filesystem_reads.max_files = 1;
    request.authority = LocalFilesystemPolicy::new(
        [directory.path().to_owned()],
        request.limits.filesystem_reads,
    )
    .expect("limited authority");

    assert!(matches!(
        process(request),
        Err(adocweave_project::ProjectError::Limit(
            ProjectLimit::Files { limit: 1 }
        ))
    ));
}
