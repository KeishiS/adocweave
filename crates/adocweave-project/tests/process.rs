use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use adocweave::NeverCancel;
use adocweave::preprocess::PreprocessErrorKind;
use adocweave_host::FilesystemReadLimits;
use adocweave_project::{
    ConfigSelection, ProjectAuthority, ProjectError, ProjectExpansionError, ProjectLimit,
    ProjectLimits, ProjectOverrides, ProjectRequest, ProjectResourceErrorCode,
    ProjectResourceFailure, ProjectResourceKind, ProjectResourceOutcome, ProjectResourceSelection,
    ProjectTarget, ProjectTargetError, TargetSelectionError, process as process_project,
};

fn process(request: ProjectRequest) -> adocweave_project::ProjectOutcome {
    process_project(request, &NeverCancel)
}

fn request(root: &Path, targets: Vec<ProjectTarget>) -> ProjectRequest {
    let filesystem_reads = FilesystemReadLimits::default();
    ProjectRequest {
        targets,
        sources: Vec::new(),
        config: ConfigSelection::Discover,
        overrides: ProjectOverrides::default(),
        apply_safe_fixes: false,
        resource_selection: ProjectResourceSelection {
            local_targets: true,
            stylesheets: true,
        },
        authority: ProjectAuthority::open(root.to_owned(), [root.to_owned()])
            .expect("temporary project is valid authority"),
        limits: ProjectLimits {
            max_files: filesystem_reads.max_files,
            max_resource_bytes: filesystem_reads.max_resource_bytes,
            max_read_bytes: filesystem_reads.max_total_bytes,
            max_directory_entries: 10_000,
            max_processing_iterations: 100,
            max_output_bytes: u32::MAX,
        },
    }
}

fn request_with_roots(
    root: &Path,
    roots: impl IntoIterator<Item = PathBuf>,
    targets: Vec<ProjectTarget>,
) -> ProjectRequest {
    let mut request = request(root, targets);
    request.authority =
        ProjectAuthority::open(root.to_owned(), roots).expect("project authority is valid");
    request
}

fn write(path: impl AsRef<Path>, source: &str) {
    fs::write(path, source).expect("fixture is written");
}

fn target_path_ends(target: &adocweave_project::ProjectTargetResult, suffix: &str) -> bool {
    target
        .path
        .as_ref()
        .is_some_and(|path| path.ends_with(suffix))
}

fn expansion(
    target: &adocweave_project::ProjectTargetResult,
) -> &Result<adocweave_project::ProjectExpandedAnalysis, ProjectExpansionError> {
    &target
        .analysis
        .as_ref()
        .expect("the primary source remains analyzed")
        .expanded
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
        "Included text.\n\ninclude::nested.adoc[]\n\nimage::asset.txt[]\n",
    );
    write(root.join("nested.adoc"), "Nested text.\n");
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
        .find(|target| target_path_ends(target, "guide.adoc"))
        .expect("guide is selected");
    let analysis = guide.analysis.as_ref().expect("guide is analyzed");
    assert!(
        analysis
            .expanded
            .as_ref()
            .expect("include expansion succeeds")
            .preprocessed
            .document
            .source
            .contains("Included text.")
    );
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
    let include_edges = guide
        .resources
        .iter()
        .filter(|resource| resource.kind == ProjectResourceKind::Include)
        .map(|resource| {
            (
                resource
                    .requested_by
                    .as_ref()
                    .expect("include requester")
                    .as_str(),
                resource.source_id.as_str(),
            )
        })
        .collect::<Vec<_>>();
    assert!(include_edges.contains(&("project:guide.adoc", "project:part.adoc")));
    assert!(include_edges.contains(&("project:part.adoc", "project:nested.adoc")));
    assert!(result.targets.iter().all(|target| target.analysis.is_ok()));
}

#[test]
fn invalid_configuration_reports_the_selected_safe_observation() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    let config = root.join(".adocweave.toml");
    write(&config, "not valid TOML = [\n");
    write(root.join("guide.adoc"), "text\n");

    let error = process(request(
        root,
        vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
    ))
    .expect_err("configuration is invalid");

    assert_eq!(
        error
            .repair_candidate()
            .map(|candidate| candidate.path.as_path()),
        Some(config.as_path())
    );
    assert!(matches!(
        error,
        ProjectError::Config(ref config_error)
            if config_error.path == config
    ));
}

#[test]
fn invalid_configuration_keeps_the_observation_from_the_failed_request() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    let config = root.join(".adocweave.toml");
    let invalid = "not valid TOML = [\n";
    write(&config, invalid);
    write(root.join("guide.adoc"), "text\n");
    let authority = ProjectAuthority::open(root, [root.to_owned()]).expect("project authority");
    let mut request = request(root, vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))]);
    request.authority = authority.clone();

    let error = process(request).expect_err("configuration is invalid");
    let candidate = error.repair_candidate().expect("repair candidate").clone();
    assert_eq!(
        candidate.observation,
        adocweave_project::ProjectResourceObservation::from_bytes(invalid.as_bytes())
    );

    write(&config, "schema-version = 2\n");
    let mut observation = authority
        .observation_access()
        .session()
        .expect("observation session");
    assert_ne!(
        observation.observe(&candidate.path, candidate.kind),
        candidate.observation
    );
}

#[test]
fn existence_observation_does_not_read_a_large_local_target_body() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    let asset = root.join("large.bin");
    fs::File::create(&asset)
        .expect("large asset")
        .set_len(11 * 1024 * 1024)
        .expect("large asset length");
    let authority = ProjectAuthority::open(root, [root.to_owned()]).expect("project authority");
    let mut observation = authority
        .observation_access()
        .session()
        .expect("observation session");

    assert_eq!(
        observation.observe(&asset, adocweave_project::ProjectObservationKind::Existence),
        adocweave_project::ProjectResourceObservation::present()
    );
}

#[test]
fn unselected_local_targets_do_not_expand_the_include_roots() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    fs::create_dir(root.join("docs")).expect("document directory");
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[local-targets]\nenabled = true\nproject-root = \".\"\n",
    );
    write(root.join("docs/guide.adoc"), "include::../part.adoc[]\n");
    write(root.join("part.adoc"), "part\n");

    let mut unselected = request(
        root,
        vec![ProjectTarget::Path(PathBuf::from("docs/guide.adoc"))],
    );
    unselected.resource_selection.local_targets = false;
    let result = process(unselected).expect("project processing remains target-local");
    assert!(expansion(&result.targets[0]).is_err());

    let selected = process(request(
        root,
        vec![ProjectTarget::Path(PathBuf::from("docs/guide.adoc"))],
    ))
    .expect("local-target checking may use its configured root");
    assert!(selected.targets[0].analysis.is_ok());
}

#[test]
fn rejected_local_targets_from_different_sources_have_distinct_ids() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    for name in ["one", "two"] {
        fs::create_dir(root.join(name)).expect("document directory");
        write(
            root.join(name).join("guide.adoc"),
            "xref:../../outside.adoc[Outside]\n",
        );
    }
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[local-targets]\nenabled = true\nproject-root = \".\"\n",
    );

    let result = process(request(
        root,
        vec![
            ProjectTarget::Path(PathBuf::from("one/guide.adoc")),
            ProjectTarget::Path(PathBuf::from("two/guide.adoc")),
        ],
    ))
    .expect("outside targets remain target-local");
    let ids = result
        .targets
        .iter()
        .flat_map(|target| &target.resources)
        .filter(|resource| resource.kind == ProjectResourceKind::LocalTarget)
        .map(|resource| resource.source_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids[0], ids[1]);
    assert!(ids.iter().all(|id| id.starts_with("local-target:")));
}

#[test]
fn missing_primary_is_confined_to_one_target() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let missing = directory.path().join("missing.adoc");
    let result = process(request(
        directory.path(),
        vec![ProjectTarget::Path(PathBuf::from("missing.adoc"))],
    ))
    .expect("request remains coherent");
    assert!(matches!(
        result.targets[0].analysis,
        Err(ProjectTargetError::Read(ref error)) if error.code == adocweave_project::ProjectResourceErrorCode::Missing
    ));
    assert!(result.targets[0].resources.iter().any(|resource| {
        resource.outcome == ProjectResourceOutcome::Missing
            && resource
                .observation
                .as_ref()
                .map(|value| value.path.as_path())
                == Some(missing.as_path())
    }));
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
        expansion(&result.targets[0]),
        Err(ProjectExpansionError::Incomplete(
            ProjectLimit::ProcessingIterations { limit: 1 }
        ))
    ));
    assert_eq!(
        result.targets[0]
            .analysis
            .as_ref()
            .expect("primary analysis remains available")
            .primary
            .source(),
        "include::part.adoc[]\n"
    );
}

#[test]
fn missing_include_keeps_the_primary_analysis() {
    let directory = tempfile::tempdir().expect("temporary directory");
    write(
        directory.path().join("guide.adoc"),
        "= Guide\n\ninclude::missing.adoc[]\n",
    );
    let mut request = request(
        directory.path(),
        vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
    );
    request.config = ConfigSelection::Disabled;
    request.overrides.include = Some(true);

    let result = process(request).expect("missing include remains target-local");
    let target = &result.targets[0];
    let analysis = target
        .analysis
        .as_ref()
        .expect("the primary source remains analyzed");
    assert_eq!(
        analysis.primary.source(),
        "= Guide\n\ninclude::missing.adoc[]\n"
    );
    let error = match &analysis.expanded {
        Err(ProjectExpansionError::Preprocess(error)) => error,
        other => panic!("unexpected expansion result: {other:?}"),
    };
    assert_eq!(error.kind, PreprocessErrorKind::MissingResource);
    assert_eq!(error.source_id.as_ref(), Some(&target.source_id));
    assert_eq!(error.range.start().to_usize(), 9);
    assert_eq!(error.range.end().to_usize(), 33);
    assert_eq!(error.requested_target.as_deref(), Some("missing.adoc"));
    assert_eq!(
        target.source.as_deref(),
        Some("= Guide\n\ninclude::missing.adoc[]\n")
    );
    assert!(target.write.is_none());
    assert!(target.resources.iter().any(|resource| {
        resource.kind == ProjectResourceKind::Include
            && resource.outcome == ProjectResourceOutcome::Missing
    }));
}

#[test]
fn include_outside_configured_roots_keeps_the_primary_analysis() {
    let directory = tempfile::tempdir().expect("temporary directory");
    fs::create_dir(directory.path().join("includes")).expect("include directory");
    write(
        directory.path().join(".adocweave.toml"),
        "schema-version = 2\n[resources]\ninclude = true\nroots = [\"includes\"]\n",
    );
    write(
        directory.path().join("guide.adoc"),
        "include::outside.adoc[]\n",
    );
    write(directory.path().join("outside.adoc"), "outside\n");

    let result = process(request(
        directory.path(),
        vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
    ))
    .expect("rejected include remains target-local");
    let target = &result.targets[0];
    let analysis = target
        .analysis
        .as_ref()
        .expect("the primary source remains analyzed");
    assert_eq!(analysis.primary.source(), "include::outside.adoc[]\n");
    assert!(matches!(
        analysis.expanded,
        Err(ProjectExpansionError::Resource(ref error))
            if error.code == ProjectResourceErrorCode::OutsideAuthority
    ));
    assert!(target.write.is_none());
    assert!(target.resources.iter().any(|resource| {
        resource.kind == ProjectResourceKind::Include
            && matches!(
                resource.outcome,
                ProjectResourceOutcome::Failed(ProjectResourceFailure::Rejected(ref error))
                    if error.code == ProjectResourceErrorCode::OutsideAuthority
            )
    }));
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
    assert!(expansion(target).is_err());
    let include = target
        .resources
        .iter()
        .find(|resource| resource.kind == ProjectResourceKind::Include)
        .expect("rejected include");
    assert!(matches!(
        include.outcome,
        ProjectResourceOutcome::Failed(ProjectResourceFailure::Rejected(_))
    ));
    let observation = include
        .observation
        .as_ref()
        .expect("safe repair observation");
    assert_eq!(observation.path, directory.path().join("linked.adoc"));
    assert_eq!(
        observation.kind,
        adocweave_project::ProjectObservationKind::Contents
    );
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
    request.limits.max_files = 1;

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
    assert_eq!(result.usage.read_operations, 3);
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
    assert!(target_path_ends(&workspace.targets[0], "kept.adoc"));

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
fn selector_permutations_duplicates_and_overlaps_have_one_stable_result_set() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    fs::create_dir(root.join("nested")).expect("nested directory");
    write(root.join("a.adoc"), "a\n");
    write(root.join("nested/b.adoc"), "b\n");
    let selectors = vec![
        ProjectTarget::Path(PathBuf::from("./a.adoc")),
        ProjectTarget::Path(PathBuf::from("a.adoc")),
        ProjectTarget::Directory(PathBuf::from("nested/..")),
        ProjectTarget::Glob("./*.adoc".to_owned()),
        ProjectTarget::Workspace(PathBuf::from(".")),
    ];
    let run = |mut selectors: Vec<ProjectTarget>| {
        let first = process(request(root, selectors.clone())).expect("selectors are valid");
        selectors.reverse();
        let second = process(request(root, selectors)).expect("reversed selectors are valid");
        let summarize = |result: adocweave_project::ProjectResult| {
            (
                result
                    .targets
                    .into_iter()
                    .map(|target| target.path)
                    .collect::<Vec<_>>(),
                result.warnings,
                result.usage,
            )
        };
        assert_eq!(summarize(first), summarize(second));
    };
    run(selectors);
}

#[test]
fn selector_errors_are_stable_across_input_order() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    let selectors = vec![
        ProjectTarget::Glob("[invalid".to_owned()),
        ProjectTarget::Workspace(PathBuf::from("../outside")),
    ];
    let first = process(request(root, selectors.clone())).expect_err("selectors are invalid");
    let second = process(request(root, selectors.into_iter().rev().collect()))
        .expect_err("reversed selectors are invalid");
    assert_eq!(first.to_string(), second.to_string());
}

#[test]
fn distinct_glob_selector_count_is_bounded_before_scanning() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let targets = (0..257)
        .map(|index| ProjectTarget::Glob(format!("document-{index}-*.adoc")))
        .collect();

    assert!(matches!(
        process(request(directory.path(), targets)),
        Err(ProjectError::TargetSelection(
            TargetSelectionError::TooManyGlobs { limit: 256 }
        ))
    ));
}

#[test]
fn total_distinct_glob_pattern_bytes_are_bounded_before_compilation() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let targets = (0..128)
        .map(|index| ProjectTarget::Glob(format!("{}-{index}-*.adoc", "segment".repeat(80))))
        .collect();

    assert!(matches!(
        process(request(directory.path(), targets)),
        Err(ProjectError::TargetSelection(
            TargetSelectionError::GlobPatternBytes { limit: 65_536 }
        ))
    ));
}

#[test]
fn directory_selectors_under_independent_roots_keep_stable_authority() {
    let project = tempfile::tempdir().expect("project directory");
    let external = tempfile::tempdir().expect("external directory");
    write(project.path().join("a.adoc"), "project\n");
    write(external.path().join("b.adoc"), "external\n");
    let run = |mut targets: Vec<ProjectTarget>| {
        let mut first_request = request(project.path(), targets.clone());
        first_request.config = ConfigSelection::Disabled;
        first_request.authority = ProjectAuthority::open(
            project.path().to_owned(),
            [project.path().to_owned(), external.path().to_owned()],
        )
        .expect("independent roots are retained");
        let first = process(first_request).expect("independent scans succeed");
        targets.reverse();
        let mut second_request = request(project.path(), targets);
        second_request.config = ConfigSelection::Disabled;
        second_request.authority = ProjectAuthority::open(
            project.path().to_owned(),
            [project.path().to_owned(), external.path().to_owned()],
        )
        .expect("independent roots are retained");
        let second = process(second_request).expect("reversed independent scans succeed");
        let ids = |result: adocweave_project::ProjectResult| {
            result
                .targets
                .into_iter()
                .map(|target| target.source_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(first), ids(second));
    };
    run(vec![
        ProjectTarget::Directory(PathBuf::from(".")),
        ProjectTarget::Directory(external.path().to_owned()),
    ]);
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
    request.limits.max_directory_entries = 2;

    let result = process(request).expect("safe partial scan is returned");
    assert_eq!(
        result.warnings,
        [adocweave_project::ProjectWarning::ScanTruncated { limit: 2 }]
    );
    assert!(!result.targets.is_empty());
}

#[test]
fn scan_limits_zero_and_one_are_stable_for_duplicate_selectors() {
    for limit in [0, 1] {
        let directory = tempfile::tempdir().expect("temporary directory");
        write(directory.path().join("a.adoc"), "a\n");
        let mut project = request(
            directory.path(),
            vec![
                ProjectTarget::Directory(PathBuf::from(".")),
                ProjectTarget::Directory(PathBuf::from("./")),
            ],
        );
        project.config = ConfigSelection::Disabled;
        project.limits.max_directory_entries = limit;
        let result = process(project).expect("bounded scan returns a stable partial result");
        if limit == 0 {
            assert_eq!(
                result.warnings,
                [adocweave_project::ProjectWarning::ScanTruncated { limit }]
            );
        } else {
            assert!(result.warnings.is_empty());
        }
        assert!(result.targets.len() <= 1);
    }
}

#[test]
fn external_primary_resolves_relative_include_from_its_own_authority() {
    let project = tempfile::tempdir().expect("project directory");
    let external = tempfile::tempdir().expect("external directory");
    write(external.path().join("guide.adoc"), "include::part.adoc[]\n");
    write(external.path().join("part.adoc"), "external include\n");
    let mut request = request_with_roots(
        project.path(),
        [project.path().to_owned(), external.path().to_owned()],
        vec![ProjectTarget::Path(external.path().join("guide.adoc"))],
    );
    request.config = ConfigSelection::Disabled;
    request.overrides.include = Some(true);
    let result = process(request).expect("external primary is processed");
    assert!(
        result.targets[0]
            .analysis
            .as_ref()
            .expect("analysis succeeds")
            .expanded
            .as_ref()
            .expect("include expansion succeeds")
            .preprocessed
            .document
            .source
            .contains("external include")
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_parent_resolves_relative_include_without_lossy_paths() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let directory = tempfile::tempdir().expect("temporary directory");
    let parent = PathBuf::from(OsString::from_vec(b"chapter-\xff".to_vec()));
    fs::create_dir(directory.path().join(&parent)).expect("non-UTF-8 directory");
    write(
        directory.path().join(&parent).join("guide.adoc"),
        "include::part.adoc[]\n",
    );
    write(
        directory.path().join(&parent).join("part.adoc"),
        "non-UTF-8 parent include\n",
    );
    let mut request = request(
        directory.path(),
        vec![ProjectTarget::Path(parent.join("guide.adoc"))],
    );
    request.config = ConfigSelection::Disabled;
    request.overrides.include = Some(true);
    let result = process(request).expect("processing succeeds");
    assert!(
        result.targets[0]
            .analysis
            .as_ref()
            .expect("analysis succeeds")
            .expanded
            .as_ref()
            .expect("include expansion succeeds")
            .preprocessed
            .document
            .source
            .contains("non-UTF-8 parent include")
    );
}

#[test]
fn workspace_selector_cannot_leave_the_project_root() {
    let project = tempfile::tempdir().expect("project directory");
    let outside = tempfile::tempdir().expect("outside directory");
    let mut request = request_with_roots(
        project.path(),
        [project.path().to_owned(), outside.path().to_owned()],
        vec![ProjectTarget::Workspace(outside.path().to_owned())],
    );
    request.config = ConfigSelection::Disabled;
    let result = process(request);
    assert!(matches!(
        result,
        Err(adocweave_project::ProjectError::Authority(_))
    ));
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
    assert!(result.targets.iter().all(|target| target.analysis.is_ok()));
}

#[test]
fn directory_selection_can_inspect_an_include_before_reading_it_as_a_target() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[resources]\ninclude = true\nmax-files = 2\nmax-total-bytes = 1024\nmax-resource-bytes = 1024\n[local-targets]\nenabled = true\nproject-root = \".\"\n",
    );
    write(
        root.join("a-guide.adoc"),
        "include::z-part.adoc[]\n\nimage::z-part.adoc[]\n",
    );
    write(root.join("z-part.adoc"), "part body\n");

    let result = process(request(
        root,
        vec![ProjectTarget::Directory(PathBuf::from("."))],
    ))
    .expect("directory processing succeeds");
    assert_eq!(result.targets.len(), 2);
    assert!(result.targets.iter().all(|target| target.analysis.is_ok()));
    let part = result
        .targets
        .iter()
        .find(|target| target_path_ends(target, "z-part.adoc"))
        .expect("included part is also a target");
    assert!(part.resources.iter().any(|resource| {
        resource.kind == ProjectResourceKind::Primary
            && matches!(resource.outcome, ProjectResourceOutcome::Loaded { .. })
    }));
}

#[test]
fn one_config_scope_combines_resource_counts_across_targets() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[resources]\nmax-files = 1\nmax-total-bytes = 1024\nmax-resource-bytes = 1024\n",
    );
    write(root.join("a.adoc"), "a\n");
    write(root.join("b.adoc"), "b\n");

    let result = process(request(
        root,
        vec![ProjectTarget::Directory(PathBuf::from("."))],
    ))
    .expect("request-wide authority remains available");
    assert!(result.targets[0].analysis.is_ok());
    assert!(matches!(
        result.targets[1].analysis,
        Err(ProjectTargetError::Incomplete(ProjectLimit::Files {
            limit: 1
        }))
    ));
    assert!(matches!(
        result.targets[1]
            .resources
            .iter()
            .find(|resource| resource.kind == ProjectResourceKind::Primary)
            .expect("primary result")
            .outcome,
        ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(ProjectLimit::Files {
            limit: 1
        }))
    ));
}

#[test]
fn one_config_scope_combines_body_bytes_across_targets() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    let config = "schema-version = 2\n[resources]\nmax-files = 2\nmax-total-bytes = 6\nmax-resource-bytes = 6\n";
    write(root.join(".adocweave.toml"), config);
    write(root.join("a.adoc"), "aaaa");
    write(root.join("b.adoc"), "bbbb");

    let result = process(request(
        root,
        vec![ProjectTarget::Directory(PathBuf::from("."))],
    ))
    .expect("request-wide acquisition can exceed a configuration scope");
    assert!(result.targets[0].analysis.is_ok());
    assert!(matches!(
        result.targets[1].analysis,
        Err(ProjectTargetError::Incomplete(ProjectLimit::ReadBytes {
            limit: 6
        }))
    ));
    assert_eq!(result.usage.read_bytes, config.len() as u64 + 7);
}

#[test]
fn per_resource_scope_limit_bounds_io_and_reports_its_own_ceiling() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    let config = "schema-version = 2\n[resources]\nmax-files = 1\nmax-total-bytes = 1024\nmax-resource-bytes = 4\n";
    write(root.join(".adocweave.toml"), config);
    write(root.join("large.adoc"), &"x".repeat(64 * 1024));
    let mut request = request(root, vec![ProjectTarget::Path(PathBuf::from("large.adoc"))]);
    request.config = ConfigSelection::Explicit(PathBuf::from(".adocweave.toml"));
    request.limits.max_files = 2;

    let result = process(request).expect("configured limit remains target-local");
    assert!(matches!(
        result.targets[0].analysis,
        Err(ProjectTargetError::Incomplete(
            ProjectLimit::ResourceBytes { limit: 4 }
        ))
    ));
    assert!(result.targets[0].resources.iter().any(|resource| {
        matches!(
            resource.outcome,
            ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(
                ProjectLimit::ResourceBytes { limit: 4 }
            ))
        ) && resource.observation.is_none()
    }));
    assert_eq!(result.usage.read_bytes, config.len() as u64 + 5);
}

#[test]
fn scope_limit_is_fixed_without_rereading_one_resource_for_later_targets() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    let config = "schema-version = 2\n[resources]\nmax-files = 3\nmax-total-bytes = 1024\nmax-resource-bytes = 4\n[html]\nstylesheet-files = [\"large.css\"]\n";
    write(root.join(".adocweave.toml"), config);
    write(root.join("a.adoc"), "a\n");
    write(root.join("b.adoc"), "b\n");
    write(root.join("large.css"), &"x".repeat(64 * 1024));
    let mut request = request(
        root,
        vec![
            ProjectTarget::Path(PathBuf::from("a.adoc")),
            ProjectTarget::Path(PathBuf::from("b.adoc")),
        ],
    );
    request.config = ConfigSelection::Explicit(PathBuf::from(".adocweave.toml"));

    let result = process(request).expect("scope limit remains target-local");
    assert!(
        result
            .targets
            .iter()
            .all(|target| expansion(target).is_ok())
    );
    assert!(
        result
            .targets
            .iter()
            .all(|target| target.resources.iter().any(|resource| {
                resource.kind == ProjectResourceKind::Stylesheet
                    && matches!(
                        resource.outcome,
                        ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(
                            ProjectLimit::ResourceBytes { limit: 4 }
                        ))
                    )
            }))
    );
    assert_eq!(result.usage.read_bytes, config.len() as u64 + 9);
}

#[test]
fn simultaneous_request_resource_limit_is_cached_without_fixing_the_scope() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    let config = "schema-version = 2\n[resources]\nmax-files = 3\nmax-total-bytes = 4096\nmax-resource-bytes = 1000\n[html]\nstylesheet-files = [\"large.css\"]\n";
    write(root.join(".adocweave.toml"), config);
    write(root.join("a.adoc"), "a\n");
    write(root.join("b.adoc"), "b\n");
    write(root.join("large.css"), &"x".repeat(64 * 1024));
    let mut request = request(
        root,
        vec![
            ProjectTarget::Path(PathBuf::from("a.adoc")),
            ProjectTarget::Path(PathBuf::from("b.adoc")),
        ],
    );
    request.config = ConfigSelection::Explicit(PathBuf::from(".adocweave.toml"));
    request.limits.max_resource_bytes = 1000;

    let result = process(request).expect("resource limit remains target-local");
    assert!(
        result
            .targets
            .iter()
            .all(|target| expansion(target).is_ok())
    );
    assert!(
        result
            .targets
            .iter()
            .all(|target| target.resources.iter().any(|resource| {
                resource.kind == ProjectResourceKind::Stylesheet
                    && matches!(
                        resource.outcome,
                        ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(
                            ProjectLimit::ResourceBytes { limit: 1000 }
                        ))
                    )
            }))
    );
    assert_eq!(result.usage.read_bytes, config.len() as u64 + 1005);
}

#[cfg(unix)]
#[test]
fn cached_success_is_checked_and_charged_under_a_narrower_scope() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    fs::create_dir(root.join("a-wide")).expect("wide scope");
    fs::create_dir(root.join("z-narrow")).expect("narrow scope");
    let wide = "schema-version = 2\n[resources]\nmax-files = 2\nmax-total-bytes = 1024\nmax-resource-bytes = 512\n[html]\nstylesheet-files = [\"shared.css\"]\n";
    let narrow = "schema-version = 2\n[resources]\nmax-files = 2\nmax-total-bytes = 1024\nmax-resource-bytes = 4\n[html]\nstylesheet-files = [\"shared.css\"]\n";
    write(root.join("a-wide/.adocweave.toml"), wide);
    write(root.join("z-narrow/.adocweave.toml"), narrow);
    write(root.join("a-wide/guide.adoc"), "w\n");
    write(root.join("z-narrow/guide.adoc"), "n\n");
    write(root.join("shared.css"), "eight888");
    symlink("../shared.css", root.join("a-wide/shared.css")).expect("wide alias");
    symlink("../shared.css", root.join("z-narrow/shared.css")).expect("narrow alias");

    let result = process(request(
        root,
        vec![
            ProjectTarget::Path(PathBuf::from("a-wide/guide.adoc")),
            ProjectTarget::Path(PathBuf::from("z-narrow/guide.adoc")),
        ],
    ))
    .expect("narrow scope remains target-local");
    assert!(result.targets[0].analysis.is_ok());
    assert!(expansion(&result.targets[1]).is_ok());
    assert!(result.targets[1].resources.iter().any(|resource| {
        resource.kind == ProjectResourceKind::Stylesheet
            && matches!(
                resource.outcome,
                ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(
                    ProjectLimit::ResourceBytes { limit: 4 }
                ))
            )
    }));
    assert_eq!(
        result.usage.read_bytes,
        (wide.len() + narrow.len() + 12) as u64
    );
}

#[test]
fn request_total_limit_is_reported_when_it_has_less_read_capacity() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    let config = "schema-version = 2\n[resources]\nmax-files = 2\nmax-total-bytes = 1024\nmax-resource-bytes = 1024\n";
    write(root.join(".adocweave.toml"), config);
    write(root.join("large.adoc"), &"x".repeat(64 * 1024));
    let mut request = request(root, vec![ProjectTarget::Path(PathBuf::from("large.adoc"))]);
    request.config = ConfigSelection::Explicit(PathBuf::from(".adocweave.toml"));
    let request_limit = config.len() as u64 + 4;
    request.limits.max_read_bytes = request_limit;

    assert!(matches!(
        process(request),
        Err(ProjectError::Limit(ProjectLimit::ReadBytes { limit }))
            if limit == request_limit
    ));
}

#[test]
fn request_resource_limit_reports_its_resource_ceiling() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    write(root.join("large.adoc"), &"x".repeat(64 * 1024));
    let mut request = request(root, vec![ProjectTarget::Path(PathBuf::from("large.adoc"))]);
    request.limits.max_read_bytes = 1024;
    request.limits.max_resource_bytes = 4;

    let result = process(request).expect("resource limit remains target-local");
    assert!(matches!(
        result.targets[0].analysis,
        Err(ProjectTargetError::Incomplete(
            ProjectLimit::ResourceBytes { limit: 4 }
        ))
    ));
    assert_eq!(result.usage.read_bytes, 5);
}

#[test]
fn config_total_limit_wins_when_its_read_also_reaches_the_file_count() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    write(root.join(".adocweave.toml"), "schema-version = 2\n");
    write(root.join("guide.adoc"), "text\n");
    let mut request = request(root, vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))]);
    request.config = ConfigSelection::Explicit(PathBuf::from(".adocweave.toml"));
    request.limits.max_files = 1;
    request.limits.max_read_bytes = 4;
    request.limits.max_resource_bytes = 1024;

    assert!(matches!(
        process(request),
        Err(ProjectError::Limit(ProjectLimit::ReadBytes { limit: 4 }))
    ));
}

#[test]
fn config_resource_limit_wins_when_its_read_also_reaches_the_file_count() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    write(root.join(".adocweave.toml"), "schema-version = 2\n");
    write(root.join("guide.adoc"), "text\n");
    let mut request = request(root, vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))]);
    request.config = ConfigSelection::Explicit(PathBuf::from(".adocweave.toml"));
    request.limits.max_files = 1;
    request.limits.max_read_bytes = 1024;
    request.limits.max_resource_bytes = 4;

    assert!(matches!(
        process(request),
        Err(ProjectError::Limit(ProjectLimit::ResourceBytes {
            limit: 4
        }))
    ));
}

#[test]
fn config_file_limit_keeps_its_file_ceiling() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    write(root.join(".adocweave.toml"), "schema-version = 2\n");
    write(root.join("guide.adoc"), "text\n");
    let mut request = request(root, vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))]);
    request.config = ConfigSelection::Explicit(PathBuf::from(".adocweave.toml"));
    request.limits.max_files = 0;

    assert!(matches!(
        process(request),
        Err(ProjectError::Limit(ProjectLimit::Files { limit: 0 }))
    ));
}

#[test]
fn one_large_config_is_shared_by_one_thousand_target_results() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    let urls = (0..1_000)
        .map(|index| format!("\"https://example.test/styles/{index:04}.css\""))
        .collect::<Vec<_>>()
        .join(", ");
    let config = format!(
        "schema-version = 2\n[resources]\nmax-files = 2000\nmax-total-bytes = 1048576\nmax-resource-bytes = 1024\n[html]\nstylesheet-urls = [{urls}]\n"
    );
    write(root.join(".adocweave.toml"), &config);
    let targets = (0..1_000)
        .map(|index| {
            let name = format!("document-{index:04}.adoc");
            write(root.join(&name), "text\n");
            ProjectTarget::Path(PathBuf::from(name))
        })
        .collect();
    let mut request = request(root, targets);
    request.config = ConfigSelection::Explicit(PathBuf::from(".adocweave.toml"));
    request.limits.max_output_bytes = 128 * 1024 * 1024;

    let result = process(request).expect("large shared configuration remains bounded");
    assert_eq!(result.targets.len(), 1_000);
    assert!(result.usage.output_bytes >= config.len() as u64);
    assert!(result.usage.output_bytes < (config.len() * 2) as u64);
    let first_config = &result.targets[0].config;
    assert!(
        result
            .targets
            .iter()
            .all(|target| Arc::ptr_eq(&target.config, first_config))
    );
}

#[test]
fn distinct_configuration_scopes_share_the_output_limit() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    fs::create_dir(root.join("first")).expect("first scope");
    fs::create_dir(root.join("second")).expect("second scope");
    let urls = (0..100)
        .map(|index| format!("\"https://example.test/styles/{index:04}.css\""))
        .collect::<Vec<_>>()
        .join(", ");
    let config = format!("schema-version = 2\n[html]\nstylesheet-urls = [{urls}]\n");
    for scope in ["first", "second"] {
        write(root.join(scope).join(".adocweave.toml"), &config);
        write(root.join(scope).join("guide.adoc"), "text\n");
    }
    let mut request = request(
        root,
        vec![
            ProjectTarget::Path(PathBuf::from("first/guide.adoc")),
            ProjectTarget::Path(PathBuf::from("second/guide.adoc")),
        ],
    );
    request.limits.max_output_bytes = u32::try_from(config.len() * 2 - 1).expect("small fixture");

    assert!(matches!(
        process(request),
        Err(ProjectError::Limit(ProjectLimit::OutputBytes { .. }))
    ));
}

#[test]
fn nested_config_has_an_independent_resource_scope() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    fs::create_dir(root.join("nested")).expect("nested directory");
    let config = "schema-version = 2\n[resources]\nmax-files = 1\nmax-total-bytes = 1024\nmax-resource-bytes = 1024\n";
    write(root.join(".adocweave.toml"), config);
    write(root.join("nested/.adocweave.toml"), config);
    write(root.join("a.adoc"), "a\n");
    write(root.join("nested/b.adoc"), "b\n");

    let result = process(request(
        root,
        vec![
            ProjectTarget::Path(PathBuf::from("a.adoc")),
            ProjectTarget::Path(PathBuf::from("nested/b.adoc")),
        ],
    ))
    .expect("nested scopes are independent");
    assert!(result.targets.iter().all(|target| target.analysis.is_ok()));
}

#[test]
fn missing_resources_reserve_scope_capacity_before_acquisition() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[resources]\nmax-files = 2\nmax-total-bytes = 1024\nmax-resource-bytes = 1024\n[html]\nstylesheet-files = [\"first.css\", \"second.css\"]\n",
    );
    write(root.join("guide.adoc"), "guide\n");

    let result = process(request(
        root,
        vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
    ))
    .expect("resource failures remain target-local");
    let styles = result.targets[0]
        .resources
        .iter()
        .filter(|resource| resource.kind == ProjectResourceKind::Stylesheet)
        .map(|resource| &resource.outcome)
        .collect::<Vec<_>>();
    assert_eq!(styles.len(), 2);
    assert!(matches!(styles[0], ProjectResourceOutcome::Missing));
    assert!(matches!(
        styles[1],
        ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(ProjectLimit::Files {
            limit: 2
        }))
    ));
    assert!(expansion(&result.targets[0]).is_ok());
}

#[test]
fn include_and_local_target_limits_mark_the_target_incomplete() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[resources]\ninclude = true\nmax-files = 1\nmax-total-bytes = 1024\nmax-resource-bytes = 1024\n[local-targets]\nenabled = true\nproject-root = \".\"\n",
    );
    write(
        root.join("guide.adoc"),
        "include::part.adoc[]\n\nimage::asset.dat[]\n",
    );
    write(root.join("part.adoc"), "part\n");
    write(root.join("asset.dat"), "asset\n");

    let result = process(request(
        root,
        vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
    ))
    .expect("configured limit remains target-local");
    assert!(matches!(
        expansion(&result.targets[0]),
        Err(ProjectExpansionError::Incomplete(ProjectLimit::Files {
            limit: 1
        }))
    ));
    assert!(result.targets[0].resources.iter().any(|resource| matches!(
        resource.outcome,
        ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(ProjectLimit::Files {
            limit: 1
        }))
    )));
}

#[test]
fn local_target_limit_remains_a_projected_diagnostic() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[resources]\nmax-files = 1\nmax-total-bytes = 1024\nmax-resource-bytes = 1024\n[local-targets]\nenabled = true\nproject-root = \".\"\n",
    );
    write(root.join("guide.adoc"), "image::asset.dat[]\n");
    write(root.join("asset.dat"), "asset\n");

    let result = process(request(
        root,
        vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
    ))
    .expect("configured limit remains target-local");
    assert!(result.targets[0].analysis.is_ok());
    assert!(
        result.targets[0]
            .analysis
            .as_ref()
            .expect("local-target limit keeps the analysis usable")
            .expanded
            .as_ref()
            .expect("include expansion succeeds")
            .local_target_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.diagnostic.code.as_str() == "local-target-limit-exceeded")
    );
    assert!(result.targets[0].resources.iter().any(|resource| {
        resource.kind == ProjectResourceKind::LocalTarget
            && matches!(
                resource.outcome,
                ProjectResourceOutcome::Failed(ProjectResourceFailure::Limit(
                    ProjectLimit::Files { limit: 1 }
                ))
            )
    }));
}

#[test]
fn include_observations_share_the_local_target_inspection_limit() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[resources]\ninclude = true\nmax-files = 2\nmax-total-bytes = 1024\nmax-resource-bytes = 1024\n[local-targets]\nenabled = true\nproject-root = \".\"\n",
    );
    write(
        root.join("guide.adoc"),
        "include::part.adoc[]\n\nimage::asset.dat[]\n",
    );
    write(root.join("part.adoc"), "part\n");
    write(root.join("asset.dat"), "asset\n");

    let result = process(request(
        root,
        vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
    ))
    .expect("include and inspection limits remain target-local");
    let analysis = result.targets[0]
        .analysis
        .as_ref()
        .expect("include remains within the document-processing limit")
        .expanded
        .as_ref()
        .expect("include expansion succeeds");
    assert!(analysis.local_target_diagnostics.iter().any(|diagnostic| {
        diagnostic.diagnostic.code.as_str() == "local-target-limit-exceeded"
            && diagnostic.target == "asset.dat"
    }));
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
        result.targets[1].analysis,
        Err(ProjectTargetError::Read(ref error)) if error.code == adocweave_project::ProjectResourceErrorCode::Missing
    ));
}

#[test]
fn returned_resource_text_is_bounded_by_the_output_limit() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    fs::create_dir(root.join("styles")).expect("stylesheet directory");
    let config = "schema-version = 2\n[html]\nstylesheet-files = [\"styles/large.css\"]\n";
    write(root.join(".adocweave.toml"), config);
    write(root.join("guide.adoc"), "text\n");
    write(root.join("styles/large.css"), &"x".repeat(100));
    let mut request = request(root, vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))]);
    let output_limit = u32::try_from(config.len() + 64).expect("small fixture");
    request.limits.max_output_bytes = output_limit;

    let result = process(request).expect("request remains coherent");
    assert!(matches!(
        expansion(&result.targets[0]),
        Err(ProjectExpansionError::Incomplete(ProjectLimit::OutputBytes {
            limit
        })) if *limit == output_limit
    ));
    assert_eq!(result.targets[0].source.as_deref(), Some("text\n"));
    assert!(result.targets[0].write.is_none());
    assert!(
        result.targets[0]
            .resources
            .iter()
            .all(|resource| !matches!(resource.outcome, ProjectResourceOutcome::Loaded { .. }))
    );
    assert!(result.targets[0].resources.iter().any(|resource| matches!(
        resource.outcome,
        ProjectResourceOutcome::LoadedOmitted {
            limit: ProjectLimit::OutputBytes { limit }
        } if limit == output_limit
    )));
}

#[test]
fn include_output_limit_keeps_only_the_primary_analysis() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let primary = "include::part.adoc[]\n";
    write(directory.path().join("guide.adoc"), primary);
    write(directory.path().join("part.adoc"), &"x".repeat(100));
    let mut request = request(
        directory.path(),
        vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
    );
    request.config = ConfigSelection::Disabled;
    request.overrides.include = Some(true);
    request.limits.max_output_bytes = u32::try_from(primary.len() + 32).expect("small fixture");

    let result = process(request).expect("request remains coherent");
    let target = &result.targets[0];
    let analysis = target
        .analysis
        .as_ref()
        .expect("the primary analysis fits the output limit");
    assert_eq!(analysis.primary.source(), primary);
    assert!(matches!(
        analysis.expanded,
        Err(ProjectExpansionError::Incomplete(ProjectLimit::OutputBytes { limit }))
            if limit == u32::try_from(primary.len() + 32).expect("small fixture")
    ));
    assert!(target.write.is_none());
    assert!(target.resources.iter().any(|resource| matches!(
        resource.outcome,
        ProjectResourceOutcome::LoadedOmitted {
            limit: ProjectLimit::OutputBytes { .. }
        }
    )));
}

#[test]
fn primary_output_limit_prevents_primary_analysis() {
    let directory = tempfile::tempdir().expect("temporary directory");
    write(directory.path().join("guide.adoc"), "text\n");
    let mut request = request(
        directory.path(),
        vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
    );
    request.config = ConfigSelection::Disabled;
    request.limits.max_output_bytes = 4;

    let result = process(request).expect("request remains coherent");
    let target = &result.targets[0];
    assert!(matches!(
        target.analysis,
        Err(ProjectTargetError::Incomplete(ProjectLimit::OutputBytes {
            limit: 4
        }))
    ));
    assert!(target.source.is_none());
    assert!(target.write.is_none());
}

#[test]
fn repeated_loaded_resource_distinguishes_body_omission_from_acquisition_failure() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    let config = "schema-version = 2\n[html]\nstylesheet-files = [\"style.css\"]\n";
    write(root.join(".adocweave.toml"), config);
    write(root.join("a.adoc"), "a\n");
    write(root.join("b.adoc"), "b\n");
    write(root.join("style.css"), "0123456789");
    let mut request = request(root, vec![ProjectTarget::Directory(PathBuf::from("."))]);
    request.limits.max_output_bytes =
        u32::try_from(config.len() + 2 + 2 + 10).expect("small fixture");
    let result = process(request).expect("processing succeeds");
    let styles = result
        .targets
        .iter()
        .map(|target| {
            &target
                .resources
                .iter()
                .find(|resource| resource.kind == ProjectResourceKind::Stylesheet)
                .expect("stylesheet result")
                .outcome
        })
        .collect::<Vec<_>>();
    assert!(matches!(styles[0], ProjectResourceOutcome::Loaded { .. }));
    assert!(matches!(
        styles[1],
        ProjectResourceOutcome::LoadedOmitted { .. }
    ));
}

#[test]
fn invalid_utf8_primary_is_reported_as_unreadable() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let bad = directory.path().join("bad.adoc");
    fs::write(&bad, [0xff]).expect("binary fixture");
    let result = process(request(
        directory.path(),
        vec![ProjectTarget::Path(PathBuf::from("bad.adoc"))],
    ))
    .expect("request remains coherent");
    assert!(matches!(
        result.targets[0].analysis,
        Err(ProjectTargetError::Read(_))
    ));
    assert!(matches!(
        result.targets[0].resources[0].outcome,
        ProjectResourceOutcome::Failed(ProjectResourceFailure::Unreadable(_))
    ));
    assert_eq!(
        result.targets[0].resources[0]
            .observation
            .as_ref()
            .map(|value| value.path.as_path()),
        Some(bad.as_path())
    );
}

#[test]
fn invalid_utf8_include_is_reported_as_watchable_unreadable_input() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let bad = directory.path().join("bad.adoc");
    write(directory.path().join("guide.adoc"), "include::bad.adoc[]\n");
    fs::write(&bad, [0xff]).expect("binary include fixture");
    let mut request = request(
        directory.path(),
        vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
    );
    request.config = ConfigSelection::Disabled;
    request.overrides.include = Some(true);

    let result = process(request).expect("unreadable include remains target-local");
    let include = result.targets[0]
        .resources
        .iter()
        .find(|resource| resource.kind == ProjectResourceKind::Include)
        .expect("include observation");
    assert!(matches!(
        include.outcome,
        ProjectResourceOutcome::Failed(ProjectResourceFailure::Unreadable(_))
    ));
    assert_eq!(
        include
            .observation
            .as_ref()
            .map(|value| value.path.as_path()),
        Some(bad.as_path())
    );
}

#[test]
fn failed_body_read_does_not_replace_local_target_presence() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[local-targets]\nenabled = true\nproject-root = \".\"\n[html]\nstylesheet-files = [\"bad.dat\"]\n",
    );
    write(root.join("a-body.adoc"), "body\n");
    write(root.join("b-guide.adoc"), "image::bad.dat[]\n");
    write(root.join("c-guide.adoc"), "image::bad.dat[]\n");
    fs::write(root.join("bad.dat"), [0xff]).expect("invalid UTF-8 fixture");

    let result = process(request(
        root,
        vec![ProjectTarget::Directory(PathBuf::from("."))],
    ))
    .expect("body failures and presence observations remain separate");
    let guides = result
        .targets
        .iter()
        .filter(|target| {
            target_path_ends(target, "b-guide.adoc") || target_path_ends(target, "c-guide.adoc")
        })
        .collect::<Vec<_>>();
    assert_eq!(guides.len(), 2);
    for guide in guides {
        assert!(guide.resources.iter().any(|resource| {
            resource.kind == ProjectResourceKind::LocalTarget
                && resource.path.ends_with("bad.dat")
                && resource.outcome == ProjectResourceOutcome::Present
        }));
    }
    assert!(result.targets.iter().all(|target| {
        target.resources.iter().any(|resource| {
            resource.kind == ProjectResourceKind::Stylesheet
                && matches!(
                    resource.outcome,
                    ProjectResourceOutcome::Failed(ProjectResourceFailure::Unreadable(_))
                )
        })
    }));
}

#[test]
fn multiple_authority_roots_receive_distinct_source_ids() {
    let project = tempfile::tempdir().expect("project directory");
    let external = tempfile::tempdir().expect("external directory");
    write(project.path().join("same.adoc"), "project\n");
    write(external.path().join("same.adoc"), "external\n");
    let mut request = request_with_roots(
        project.path(),
        [project.path().to_owned(), external.path().to_owned()],
        vec![
            ProjectTarget::Path(PathBuf::from("same.adoc")),
            ProjectTarget::Path(external.path().join("same.adoc")),
        ],
    );
    request.config = ConfigSelection::Disabled;
    let result = process(request).expect("both roots are processed");
    assert_eq!(result.targets.len(), 2);
    assert_ne!(result.targets[0].source_id, result.targets[1].source_id);
}

#[test]
fn external_config_authority_does_not_grant_its_include_stylesheet_or_local_paths() {
    let project = tempfile::tempdir().expect("project directory");
    let external = tempfile::tempdir().expect("external directory");
    write(project.path().join("guide.adoc"), "text\n");
    for (name, body) in [
        (
            "include.toml",
            "schema-version = 2\n[resources]\nroots = [\"assets\"]\n",
        ),
        (
            "stylesheet.toml",
            "schema-version = 2\n[html]\nstylesheet-files = [\"style.css\"]\n",
        ),
        (
            "local.toml",
            "schema-version = 2\n[local-targets]\nenabled = true\nproject-root = \"assets\"\n",
        ),
    ] {
        let config = external.path().join(name);
        write(&config, body);
        let mut request = request_with_roots(
            project.path(),
            [project.path().to_owned(), external.path().to_owned()],
            vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
        );
        request.config = ConfigSelection::Explicit(config.clone());
        let result = process(request);
        assert!(
            matches!(&result, Err(adocweave_project::ProjectError::Authority(_))),
            "external configuration path must not grant {name} resources"
        );
        assert_eq!(
            result
                .expect_err("authority failure")
                .repair_candidate()
                .map(|candidate| candidate.path.as_path()),
            Some(config.as_path()),
            "the safely acquired configuration must remain repairable"
        );
    }
}

#[test]
fn external_config_authority_can_read_only_the_config_body() {
    let project = tempfile::tempdir().expect("project directory");
    let external = tempfile::tempdir().expect("external directory");
    write(project.path().join("guide.adoc"), "text\n");
    let config = external.path().join("adocweave.toml");
    write(&config, "schema-version = 2\n");
    let mut request = request_with_roots(
        project.path(),
        [project.path().to_owned(), external.path().to_owned()],
        vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
    );
    request.config = ConfigSelection::Explicit(config);
    let result = process(request).expect("external configuration body is readable");
    assert!(result.targets[0].analysis.is_ok());
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
fn no_symlink_path_target_rejects_an_explicit_symbolic_link() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    write(root.join("real.adoc"), "REAL\n");
    symlink("real.adoc", root.join("link.adoc")).expect("symbolic link");

    let result = process(request(
        root,
        vec![ProjectTarget::PathNoSymlinks(PathBuf::from("link.adoc"))],
    ))
    .expect("project result");
    let target = result.targets.first().expect("selected target");

    assert!(matches!(target.analysis, Err(ProjectTargetError::Read(_))));
    assert_eq!(target.resources.len(), 1);
    assert_eq!(target.resources[0].kind, ProjectResourceKind::Primary);
    let observation = target.resources[0]
        .observation
        .as_ref()
        .expect("safe repair observation");
    assert_eq!(observation.path, root.join("link.adoc"));
    assert_eq!(
        observation.kind,
        adocweave_project::ProjectObservationKind::ContentsNoSymlinks
    );
    assert!(matches!(
        target.resources[0].outcome,
        ProjectResourceOutcome::Failed(ProjectResourceFailure::Rejected(_))
    ));
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
        result.targets[0].analysis,
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
    assert!(expansion(&result.targets[0]).is_err());
}

#[cfg(unix)]
#[test]
fn broad_cached_read_cannot_bypass_a_later_confined_include_root() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    fs::create_dir(root.join("narrow")).expect("narrow directory");
    fs::create_dir(root.join("outside")).expect("outside directory");
    write(
        root.join("narrow/.adocweave.toml"),
        "schema-version = 2\n[resources]\ninclude = true\nroots = [\".\"]\n",
    );
    write(root.join("outside/secret.adoc"), "secret\n");
    symlink(
        root.join("outside/secret.adoc"),
        root.join("narrow/link.adoc"),
    )
    .expect("include symlink");
    write(root.join("narrow/z-guide.adoc"), "include::link.adoc[]\n");

    let result = process(request(
        root,
        vec![
            ProjectTarget::Path(PathBuf::from("narrow/link.adoc")),
            ProjectTarget::Path(PathBuf::from("narrow/z-guide.adoc")),
        ],
    ))
    .expect("request remains coherent");
    let guide = result
        .targets
        .iter()
        .find(|target| target_path_ends(target, "z-guide.adoc"))
        .expect("guide result");
    assert!(expansion(guide).is_err());
    assert!(guide.resources.iter().any(|resource| {
        resource.kind == ProjectResourceKind::Include
            && matches!(resource.outcome, ProjectResourceOutcome::Failed(_))
    }));
}

#[test]
fn broad_cached_local_target_cannot_bypass_a_later_confined_root() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    fs::create_dir(root.join("narrow")).expect("narrow directory");
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[local-targets]\nenabled = true\nproject-root = \".\"\n",
    );
    write(
        root.join("narrow/.adocweave.toml"),
        "schema-version = 2\n[local-targets]\nenabled = true\nproject-root = \".\"\n",
    );
    write(root.join("shared.dat"), "shared\n");
    write(root.join("a.adoc"), "image::shared.dat[]\n");
    write(root.join("narrow/z.adoc"), "image::../shared.dat[]\n");

    let result = process(request(
        root,
        vec![
            ProjectTarget::Path(PathBuf::from("a.adoc")),
            ProjectTarget::Path(PathBuf::from("narrow/z.adoc")),
        ],
    ))
    .expect("request remains coherent");
    let broad = result
        .targets
        .iter()
        .find(|target| target_path_ends(target, "a.adoc"))
        .expect("broad target");
    let narrow = result
        .targets
        .iter()
        .find(|target| target_path_ends(target, "z.adoc"))
        .expect("narrow target");
    assert!(broad.resources.iter().any(|resource| {
        resource.kind == ProjectResourceKind::LocalTarget
            && resource.outcome == ProjectResourceOutcome::Present
    }));
    assert!(narrow.resources.iter().any(|resource| {
        resource.kind == ProjectResourceKind::LocalTarget
            && matches!(
                resource.outcome,
                ProjectResourceOutcome::Failed(ProjectResourceFailure::Rejected(_))
            )
    }));
}

#[test]
fn parent_root_body_does_not_prevent_a_child_root_inspection() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    fs::create_dir(root.join("child")).expect("child directory");
    write(
        root.join("child/.adocweave.toml"),
        "schema-version = 2\n[local-targets]\nenabled = true\nproject-root = \".\"\n",
    );
    write(root.join("child/a-resource.adoc"), "resource\n");
    write(
        root.join("child/z-guide.adoc"),
        "image::a-resource.adoc[]\n",
    );

    let result = process(request(
        root,
        vec![
            ProjectTarget::Path(PathBuf::from("child/a-resource.adoc")),
            ProjectTarget::Path(PathBuf::from("child/z-guide.adoc")),
        ],
    ))
    .expect("request remains coherent");
    let guide = result
        .targets
        .iter()
        .find(|target| target_path_ends(target, "z-guide.adoc"))
        .expect("guide result");
    assert!(guide.resources.iter().any(|resource| {
        resource.kind == ProjectResourceKind::LocalTarget
            && resource.path.ends_with("a-resource.adoc")
            && resource.outcome == ProjectResourceOutcome::Present
    }));
}

#[test]
fn child_root_body_does_not_prevent_a_parent_root_inspection() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    let child = root.join("child");
    fs::create_dir(&child).expect("child directory");
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[local-targets]\nenabled = true\nproject-root = \".\"\n",
    );
    write(child.join("a-resource.adoc"), "resource\n");
    write(
        root.join("z-guide.adoc"),
        "image::child/a-resource.adoc[]\n",
    );
    let mut request = request(
        root,
        vec![
            ProjectTarget::Path(PathBuf::from("child/a-resource.adoc")),
            ProjectTarget::Path(PathBuf::from("z-guide.adoc")),
        ],
    );
    request.authority = ProjectAuthority::open(root.to_owned(), [root.to_owned(), child])
        .expect("nested roots are retained");

    let result = process(request).expect("request remains coherent");
    let guide = result
        .targets
        .iter()
        .find(|target| target_path_ends(target, "z-guide.adoc"))
        .expect("guide result");
    assert!(guide.resources.iter().any(|resource| {
        resource.kind == ProjectResourceKind::LocalTarget
            && resource.path.ends_with("a-resource.adoc")
            && resource.outcome == ProjectResourceOutcome::Present
    }));
}

#[cfg(unix)]
#[test]
fn canonical_cached_body_cannot_hide_an_outside_local_target_request() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    fs::create_dir(root.join("outside")).expect("outside directory");
    fs::create_dir(root.join("narrow")).expect("narrow directory");
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[html]\nstylesheet-files = [\"outside/alias.dat\"]\n",
    );
    write(
        root.join("narrow/.adocweave.toml"),
        "schema-version = 2\n[local-targets]\nenabled = true\nproject-root = \".\"\n",
    );
    write(root.join("narrow/actual.dat"), "actual\n");
    symlink(
        PathBuf::from("../narrow/actual.dat"),
        root.join("outside/alias.dat"),
    )
    .expect("reverse symlink");
    write(root.join("a.adoc"), "a\n");
    write(
        root.join("narrow/z.adoc"),
        "image::../outside/alias.dat[]\n",
    );

    let result = process(request(
        root,
        vec![
            ProjectTarget::Path(PathBuf::from("a.adoc")),
            ProjectTarget::Path(PathBuf::from("narrow/z.adoc")),
        ],
    ))
    .expect("request remains coherent");
    let broad = result
        .targets
        .iter()
        .find(|target| target_path_ends(target, "a.adoc"))
        .expect("broad target");
    let narrow = result
        .targets
        .iter()
        .find(|target| target_path_ends(target, "z.adoc"))
        .expect("narrow target");
    assert!(broad.resources.iter().any(|resource| {
        resource.kind == ProjectResourceKind::Stylesheet
            && matches!(resource.outcome, ProjectResourceOutcome::Loaded { .. })
    }));
    assert!(narrow.resources.iter().any(|resource| {
        resource.kind == ProjectResourceKind::LocalTarget
            && matches!(
                resource.outcome,
                ProjectResourceOutcome::Failed(ProjectResourceFailure::Rejected(_))
            )
    }));
}

#[cfg(unix)]
#[test]
fn same_authority_reuses_loaded_present_missing_and_failed_observations() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary directory");
    let outside = tempfile::tempdir().expect("outside directory");
    let root = directory.path();
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[local-targets]\nenabled = true\nproject-root = \".\"\n[html]\nstylesheet-files = [\"loaded.dat\"]\n",
    );
    write(root.join("loaded.dat"), "loaded\n");
    write(root.join("present.dat"), "present\n");
    write(outside.path().join("failed.dat"), "failed\n");
    symlink(outside.path().join("failed.dat"), root.join("failed.dat"))
        .expect("failed local target symlink");
    let references =
        "image::loaded.dat[]\nimage::present.dat[]\nimage::missing.dat[]\nimage::failed.dat[]\n";
    write(root.join("a.adoc"), references);
    write(root.join("b.adoc"), references);

    let result = process(request(
        root,
        vec![
            ProjectTarget::Path(PathBuf::from("a.adoc")),
            ProjectTarget::Path(PathBuf::from("b.adoc")),
        ],
    ))
    .expect("same-authority observations are reusable");
    let second = result
        .targets
        .iter()
        .find(|target| target_path_ends(target, "b.adoc"))
        .expect("second target");
    let local = |name: &str| {
        &second
            .resources
            .iter()
            .find(|resource| {
                resource.kind == ProjectResourceKind::LocalTarget && resource.path.ends_with(name)
            })
            .unwrap_or_else(|| panic!("local target result for {name}"))
            .outcome
    };
    assert_eq!(local("loaded.dat"), &ProjectResourceOutcome::Present);
    assert_eq!(local("present.dat"), &ProjectResourceOutcome::Present);
    assert_eq!(local("missing.dat"), &ProjectResourceOutcome::Missing);
    assert!(matches!(
        local("failed.dat"),
        ProjectResourceOutcome::Failed(ProjectResourceFailure::Rejected(_))
    ));
}

#[cfg(unix)]
#[test]
fn normal_cached_read_cannot_be_reused_as_no_symlink_config_read() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path();
    fs::create_dir(root.join("nested")).expect("nested directory");
    write(
        root.join(".adocweave.toml"),
        "schema-version = 2\n[html]\nstylesheet-files = [\"nested/.adocweave.toml\"]\n",
    );
    write(root.join("actual.toml"), "schema-version = 2\n");
    symlink(
        root.join("actual.toml"),
        root.join("nested/.adocweave.toml"),
    )
    .expect("configuration symlink");
    write(root.join("a.adoc"), "a\n");
    write(root.join("nested/z.adoc"), "z\n");

    let result = process(request(
        root,
        vec![
            ProjectTarget::Path(PathBuf::from("a.adoc")),
            ProjectTarget::Path(PathBuf::from("nested/z.adoc")),
        ],
    ));
    assert!(matches!(
        result,
        Err(adocweave_project::ProjectError::Authority(_))
    ));
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
    let stylesheet = resources
        .iter()
        .find(|resource| resource.kind == ProjectResourceKind::Stylesheet)
        .expect("stylesheet failure");
    assert!(matches!(
        stylesheet.outcome,
        ProjectResourceOutcome::Failed(_)
    ));
    let stylesheet_observation = stylesheet
        .observation
        .as_ref()
        .expect("stylesheet repair observation");
    assert_eq!(stylesheet_observation.path, root.join("style.css"));
    assert_eq!(
        stylesheet_observation.kind,
        adocweave_project::ProjectObservationKind::Contents
    );

    let local_target = resources
        .iter()
        .find(|resource| resource.kind == ProjectResourceKind::LocalTarget)
        .expect("local target failure");
    assert!(matches!(
        local_target.outcome,
        ProjectResourceOutcome::Failed(_)
    ));
    let local_target_observation = local_target
        .observation
        .as_ref()
        .expect("local target repair observation");
    assert_eq!(local_target_observation.path, root.join("asset.dat"));
    assert_eq!(
        local_target_observation.kind,
        adocweave_project::ProjectObservationKind::Existence
    );
}
