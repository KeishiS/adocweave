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

#[test]
fn missing_config_discovery_is_fixed_and_charged_once() {
    let directory = tempfile::tempdir().expect("temporary directory");
    write(directory.path().join("a.adoc"), "a\n");
    write(directory.path().join("b.adoc"), "b\n");
    let result = process(request(
        directory.path(),
        vec![
            ProjectTarget::Path(PathBuf::from("a.adoc")),
            ProjectTarget::Path(PathBuf::from("b.adoc")),
        ],
    ))
    .expect("processing succeeds");
    assert_eq!(result.usage.filesystem.read_operations, 3);
}

#[test]
fn workspace_excludes_apply_before_scan_but_explicit_directory_keeps_files() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    fs::create_dir(root.join("generated")).expect("generated directory");
    write(root.join("kept.adoc"), "kept\n");
    write(root.join("generated/hidden.adoc"), "hidden\n");
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[workspace.scan]\nexclude = [\"generated\"]\n",
    );

    let workspace = process(request(
        root,
        vec![ProjectTarget::Workspace(PathBuf::from("."))],
    ))
    .expect("workspace scan succeeds");
    assert_eq!(workspace.targets.len(), 1);
    assert!(workspace.targets[0].path.ends_with("kept.adoc"));

    let explicit = process(request(
        root,
        vec![ProjectTarget::Directory(PathBuf::from("."))],
    ))
    .expect("explicit directory succeeds");
    assert_eq!(explicit.targets.len(), 2);
}

#[test]
fn target_results_are_stable_across_selector_order() {
    let directory = tempfile::tempdir().expect("temporary directory");
    write(directory.path().join("a.adoc"), "a\n");
    write(directory.path().join("b.adoc"), "b\n");
    let targets = |reverse| {
        let mut targets = vec![
            ProjectTarget::Path(PathBuf::from("b.adoc")),
            ProjectTarget::Path(PathBuf::from("a.adoc")),
        ];
        if reverse {
            targets.reverse();
        }
        process(request(directory.path(), targets))
            .expect("processing succeeds")
            .targets
            .into_iter()
            .map(|target| (target.source_id, target.path))
            .collect::<Vec<_>>()
    };
    assert_eq!(targets(false), targets(true));
}

#[test]
fn bounded_workspace_scan_reports_partial_results() {
    let directory = tempfile::tempdir().expect("temporary directory");
    fs::create_dir(directory.path().join("nested")).expect("nested directory");
    write(directory.path().join("a.adoc"), "a\n");
    write(directory.path().join("nested/b.adoc"), "b\n");
    let mut request = request(
        directory.path(),
        vec![ProjectTarget::Workspace(PathBuf::from("."))],
    );
    request.config = ConfigSelection::Disabled;
    request.limits.max_directory_entries = 1;

    let result = process(request).expect("safe partial scan is returned");
    assert_eq!(
        result.warnings,
        [adocweave_project::ProjectWarning::ScanTruncated { limit: 1 }]
    );
}

#[test]
fn target_outside_authority_is_rejected_before_reading() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let outside = tempfile::tempdir().expect("outside directory");
    write(outside.path().join("outside.adoc"), "outside\n");
    let result = process(request(
        directory.path(),
        vec![ProjectTarget::Path(outside.path().join("outside.adoc"))],
    ));
    assert!(matches!(
        result,
        Err(adocweave_project::ProjectError::Authority(_))
    ));
}

#[test]
fn local_target_observation_can_later_be_read_as_a_primary() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[local-targets]\nenabled = true\nproject-root = \".\"\n",
    );
    write(root.join("a-guide.adoc"), "image::z-resource.adoc[]\n");
    write(root.join("z-resource.adoc"), "resource body\n");

    let result = process(request(
        root,
        vec![ProjectTarget::Directory(PathBuf::from("."))],
    ))
    .expect("inspection and later read coexist");
    assert_eq!(result.targets.len(), 2);
    assert!(result.targets.iter().all(|target| target.outcome.is_ok()));
}

#[test]
fn failed_local_target_observation_is_reused_by_a_later_primary() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[local-targets]\nenabled = true\nproject-root = \".\"\n",
    );
    write(root.join("a-guide.adoc"), "image::z-missing.adoc[]\n");
    let result = process(request(
        root,
        vec![
            ProjectTarget::Path(PathBuf::from("a-guide.adoc")),
            ProjectTarget::Path(PathBuf::from("z-missing.adoc")),
        ],
    ))
    .expect("missing observation remains a target-local failure");
    assert_eq!(result.targets.len(), 2);
    assert!(matches!(
        result.targets[1].outcome,
        Err(ProjectTargetError::Read(ResourceError::Missing(_)))
    ));
}

#[test]
fn returned_resource_text_is_bounded_by_the_output_limit() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    fs::create_dir(root.join("styles")).expect("stylesheet directory");
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[html]\nstylesheet-files = [\"styles/large.css\"]\n",
    );
    write(root.join("guide.adoc"), "text\n");
    write(root.join("styles/large.css"), &"x".repeat(100));
    let mut request = request(root, vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))]);
    request.limits.output.max_output_bytes = 64;

    let result = process(request).expect("request remains coherent");
    assert!(matches!(
        result.targets[0].outcome,
        Err(ProjectTargetError::Incomplete(ProjectLimit::OutputBytes {
            limit: 64
        }))
    ));
    assert!(
        result.targets[0]
            .resources
            .iter()
            .all(|resource| !matches!(resource.outcome, ProjectResourceOutcome::Loaded { .. }))
    );
}

#[test]
fn invalid_utf8_primary_is_reported_as_unreadable() {
    let directory = tempfile::tempdir().expect("temporary directory");
    fs::write(directory.path().join("bad.adoc"), [0xff]).expect("binary fixture");
    let result = process(request(
        directory.path(),
        vec![ProjectTarget::Path(PathBuf::from("bad.adoc"))],
    ))
    .expect("request remains coherent");
    assert!(matches!(
        result.targets[0].outcome,
        Err(ProjectTargetError::Read(_))
    ));
    assert!(matches!(
        result.targets[0].resources[0].outcome,
        ProjectResourceOutcome::Failed(ProjectResourceFailure::Unreadable(_))
    ));
}

#[test]
fn multiple_authority_roots_receive_distinct_source_ids() {
    let project = tempfile::tempdir().expect("project directory");
    let external = tempfile::tempdir().expect("external directory");
    write(project.path().join("same.adoc"), "project\n");
    write(external.path().join("same.adoc"), "external\n");
    let filesystem_reads = FilesystemReadLimits::default();
    let result = process(ProjectRequest {
        project_root: project.path().to_owned(),
        targets: vec![
            ProjectTarget::Path(PathBuf::from("same.adoc")),
            ProjectTarget::Path(external.path().join("same.adoc")),
        ],
        config: ConfigSelection::Disabled,
        overrides: ProjectOverrides::default(),
        authority: LocalFilesystemPolicy::new(
            [project.path().to_owned(), external.path().to_owned()],
            filesystem_reads,
        )
        .expect("two roots are retained"),
        limits: ProjectLimits {
            filesystem_reads,
            max_directory_entries: 100,
            max_processing_iterations: 10,
            output: OutputLimits::default(),
        },
    })
    .expect("both roots are processed");
    assert_eq!(result.targets.len(), 2);
    assert_ne!(result.targets[0].source_id, result.targets[1].source_id);
}

#[cfg(unix)]
#[test]
fn non_utf8_path_has_a_distinct_stable_source_id() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let directory = tempfile::tempdir().expect("temporary directory");
    let invalid = PathBuf::from(OsString::from_vec(b"bad-\xff.adoc".to_vec()));
    let lossy = PathBuf::from("bad-�.adoc");
    write(directory.path().join(&invalid), "invalid name\n");
    write(directory.path().join(&lossy), "utf8 name\n");
    let result = process(request(
        directory.path(),
        vec![ProjectTarget::Path(invalid), ProjectTarget::Path(lossy)],
    ))
    .expect("both path encodings are processed");
    assert_eq!(result.targets.len(), 2);
    assert_ne!(result.targets[0].source_id, result.targets[1].source_id);
}

#[cfg(unix)]
#[test]
fn explicit_symlinked_config_is_rejected() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary directory");
    let outside = tempfile::tempdir().expect("outside directory");
    write(directory.path().join("guide.adoc"), "text\n");
    write(outside.path().join("config.toml"), "schema-version = 2\n");
    symlink(
        outside.path().join("config.toml"),
        directory.path().join("linked.toml"),
    )
    .expect("config symlink");
    let mut request = request(
        directory.path(),
        vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
    );
    request.config = ConfigSelection::Explicit(PathBuf::from("linked.toml"));
    assert!(process(request).is_err());
}

#[cfg(unix)]
#[test]
fn unreadable_primary_retains_permission_failure() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("private.adoc");
    write(&path, "private\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("remove read access");
    let result = process(request(
        directory.path(),
        vec![ProjectTarget::Path(PathBuf::from("private.adoc"))],
    ));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("restore read access");
    let result = result.expect("request remains coherent");
    assert!(matches!(
        result.targets[0].outcome,
        Err(ProjectTargetError::Read(_))
    ));
}

#[cfg(unix)]
#[test]
fn configured_include_root_cannot_cross_a_symlink() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    fs::create_dir(root.join("real")).expect("real include directory");
    write(root.join("real/part.adoc"), "secret\n");
    symlink(root.join("real"), root.join("linked")).expect("root symlink");
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[resources]\ninclude = true\nroots = [\"linked\"]\n",
    );
    write(root.join("guide.adoc"), "include::linked/part.adoc[]\n");

    let result = process(request(
        root,
        vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
    ))
    .expect("request remains coherent");
    assert!(result.targets[0].outcome.is_err());
}

#[cfg(unix)]
#[test]
fn stylesheet_and_local_target_symlinks_are_retained_as_failures() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary directory");
    let outside = tempfile::tempdir().expect("outside directory");
    let root = directory.path();
    write(outside.path().join("style.css"), "body {}\n");
    write(outside.path().join("asset.dat"), "asset\n");
    symlink(outside.path().join("style.css"), root.join("style.css")).expect("stylesheet symlink");
    symlink(outside.path().join("asset.dat"), root.join("asset.dat")).expect("asset symlink");
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[local-targets]\nenabled = true\nproject-root = \".\"\n[html]\nstylesheet-files = [\"style.css\"]\n",
    );
    write(root.join("guide.adoc"), "image::asset.dat[]\n");

    let result = process(request(
        root,
        vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
    ))
    .expect("request remains coherent");
    let resources = &result.targets[0].resources;
    for kind in [
        ProjectResourceKind::Stylesheet,
        ProjectResourceKind::LocalTarget,
    ] {
        assert!(resources.iter().any(|resource| {
            resource.kind == kind && matches!(resource.outcome, ProjectResourceOutcome::Failed(_))
        }));
    }
}
