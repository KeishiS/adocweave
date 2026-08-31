use adocweave_core::{NeverCancel, SourceId};
use adocweave_project::{
    ProjectAuthority, ProjectConfigOverrides, ProjectConfigRequest, ProjectConfigSelection,
    ProjectError, ProjectLimit, ProjectLimits, ProjectRequest, ProjectResourceKind,
    ProjectResourceLimits, ProjectResourceOrigin, ProjectResourceSelection, ProjectSource,
    ProjectTarget, process, resolve_config,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn request_with(targets: Vec<ProjectTarget>) -> ProjectRequest {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let authority = ProjectAuthority::open(project_root.clone(), [project_root.clone()])
        .expect("the crate directory is an existing filesystem authority");
    ProjectRequest {
        targets,
        sources: Vec::new(),
        config: ProjectConfigSelection::Discover,
        overrides: ProjectConfigOverrides::default(),
        apply_safe_fixes: false,
        resource_selection: Default::default(),
        authority,
        limits: ProjectLimits {
            resources: ProjectResourceLimits {
                max_files: 100_000,
                max_total_bytes: 50 * 1024 * 1024,
                max_resource_bytes: 10 * 1024 * 1024,
            },
            max_directory_entries: 100_000,
            max_processing_iterations: 100_000,
            max_output_bytes: u32::MAX,
        },
    }
}

#[test]
fn one_owned_request_accepts_every_target_form() {
    let path = PathBuf::from("README.adoc");
    let directory = PathBuf::from("docs");
    let glob = String::from("docs/**/*.adoc");
    let request = request_with(vec![
        ProjectTarget::Path(path.clone()),
        ProjectTarget::Directory(directory.clone()),
        ProjectTarget::Glob(glob.clone()),
    ]);
    drop((path, directory, glob));

    assert_eq!(request.targets.len(), 3);
    assert!(matches!(request.targets[0], ProjectTarget::Path(_)));
    assert!(matches!(request.targets[1], ProjectTarget::Directory(_)));
    assert!(matches!(request.targets[2], ProjectTarget::Glob(_)));
}

#[test]
fn in_memory_source_is_owned_by_the_request() {
    let mut request = request_with(Vec::new());
    let id = SourceId::new("stdin");
    request.sources.push(ProjectSource::new(
        id.clone(),
        request.authority.project_root().join("stdin.adoc"),
        "= Standard input\n",
    ));
    request.targets.push(ProjectTarget::Source(id));
    request.config = ProjectConfigSelection::Disabled;
    let result = process(request, &NeverCancel).expect("memory source is processed");
    assert_eq!(
        result.targets[0].source.as_deref(),
        Some("= Standard input\n")
    );
    assert!(result.targets[0].write.is_none());
}

#[test]
fn file_write_capability_is_bound_to_the_observed_contents() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("guide.adoc");
    fs::write(&path, "original\n").expect("source");
    let request = || ProjectRequest {
        targets: vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
        sources: Vec::new(),
        config: ProjectConfigSelection::Disabled,
        overrides: ProjectConfigOverrides::default(),
        apply_safe_fixes: false,
        resource_selection: Default::default(),
        authority: ProjectAuthority::open(
            directory.path().to_owned(),
            [directory.path().to_owned()],
        )
        .expect("project authority"),
        limits: ProjectLimits::default(),
    };

    let result = process(request(), &NeverCancel).expect("project processing");
    let capability = result.targets.into_iter().next().unwrap().write.unwrap();
    assert_eq!(capability.contents_match(), Ok(true));
    assert_eq!(capability.replace_after_recheck(b"updated\n"), Ok(true));
    assert_eq!(fs::read_to_string(&path).unwrap(), "updated\n");

    let result = process(request(), &NeverCancel).expect("second project processing");
    let capability = result.targets.into_iter().next().unwrap().write.unwrap();
    fs::write(&path, "concurrent\n").expect("concurrent update");
    assert_eq!(capability.replace_after_recheck(b"rejected\n"), Ok(false));
    assert_eq!(fs::read_to_string(&path).unwrap(), "concurrent\n");
}

#[test]
fn safe_fixes_return_the_original_and_reanalyze_the_replacement() {
    let directory = tempfile::tempdir().expect("temporary directory");
    fs::write(directory.path().join("guide.adoc"), "text  \n").expect("source");
    let make_request = |apply_safe_fixes| ProjectRequest {
        targets: vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
        sources: Vec::new(),
        config: ProjectConfigSelection::Disabled,
        overrides: ProjectConfigOverrides::default(),
        apply_safe_fixes,
        resource_selection: Default::default(),
        authority: ProjectAuthority::open(
            directory.path().to_owned(),
            [directory.path().to_owned()],
        )
        .expect("project authority"),
        limits: ProjectLimits::default(),
    };

    let unchanged = process(make_request(false), &NeverCancel).expect("project processing");
    assert!(unchanged.targets[0].replacement_source.is_none());

    let fixed = process(make_request(true), &NeverCancel).expect("fixed project processing");
    let target = &fixed.targets[0];
    assert_eq!(target.source.as_deref(), Some("text  \n"));
    assert_eq!(target.replacement_source.as_deref(), Some("text\n"));
    assert!(
        target
            .analysis
            .as_ref()
            .expect("fixed analysis")
            .primary
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.code.as_str() != "trailing-whitespace")
    );
}

#[test]
fn contracts_have_no_caller_borrowed_lifetime() {
    fn owns_request(_: ProjectRequest) -> Box<dyn std::any::Any + Send> {
        Box::new("request consumed")
    }

    let owned = owns_request(request_with(vec![ProjectTarget::Path(PathBuf::from(
        "guide.adoc",
    ))]));
    assert!(owned.is::<&'static str>());
}

#[test]
fn public_requests_and_results_are_send_sync_static() {
    fn assert_contract<T: Send + Sync + 'static>() {}
    assert_contract::<ProjectAuthority>();
    assert_contract::<ProjectRequest>();
    assert_contract::<adocweave_project::ProjectResult>();
    assert_contract::<adocweave_project::ProjectConfigResult>();
}

#[test]
fn relative_project_root_is_rejected_even_with_multiple_roots() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    assert!(ProjectAuthority::open(PathBuf::from("relative"), [root]).is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_project_root_uses_the_opened_canonical_identity() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary directory");
    let actual = directory.path().join("actual");
    let linked = directory.path().join("linked");
    fs::create_dir(&actual).expect("actual project");
    fs::write(actual.join("guide.adoc"), "linked project\n").expect("source");
    symlink(&actual, &linked).expect("project symlink");
    let authority =
        ProjectAuthority::open(linked.clone(), [linked]).expect("symlinked root is accepted");
    assert_eq!(authority.project_root(), actual.canonicalize().unwrap());

    let result = process(
        ProjectRequest {
            targets: vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
            sources: Vec::new(),
            config: ProjectConfigSelection::Disabled,
            overrides: ProjectConfigOverrides::default(),
            apply_safe_fixes: false,
            resource_selection: Default::default(),
            authority,
            limits: request_with(Vec::new()).limits,
        },
        &NeverCancel,
    )
    .expect("source is read through retained identity");
    assert_eq!(
        result.targets[0].source.as_deref(),
        Some("linked project\n")
    );
}

#[cfg(target_os = "macos")]
#[test]
fn authored_var_path_is_processed_through_its_canonical_project_authority() {
    let directory = tempfile::Builder::new()
        .prefix("adocweave-project-")
        .tempdir_in("/var/tmp")
        .expect("temporary project root");
    let canonical = directory
        .path()
        .canonicalize()
        .expect("canonical project root");
    let authored = std::path::Path::new("/var").join(
        canonical
            .strip_prefix("/private/var")
            .expect("macOS /var resolves below /private/var"),
    );
    fs::write(authored.join("guide.adoc"), "authored project\n").expect("project source");
    let authority = ProjectAuthority::open(authored.clone(), [authored.clone()])
        .expect("authored spelling establishes the canonical authority");

    let result = process(
        ProjectRequest {
            targets: vec![
                ProjectTarget::Path(authored.join("guide.adoc")),
                ProjectTarget::Path(canonical.join("guide.adoc")),
            ],
            sources: Vec::new(),
            config: ProjectConfigSelection::Disabled,
            overrides: ProjectConfigOverrides::default(),
            apply_safe_fixes: false,
            resource_selection: Default::default(),
            authority,
            limits: ProjectLimits::default(),
        },
        &NeverCancel,
    )
    .expect("authored source path is processed");
    assert_eq!(result.targets.len(), 1);
    assert_eq!(
        result.targets[0].source.as_deref(),
        Some("authored project\n")
    );
    assert_eq!(result.targets[0].source_id.as_str(), "project:guide.adoc");
}

#[test]
fn configuration_can_be_resolved_without_processing_a_target() {
    let directory = tempfile::tempdir().expect("temporary directory");
    fs::write(
        directory.path().join(".adocweave.toml"),
        "schema-version = 2\n[resources]\nmax-files = 7\nmax-total-bytes = 99\nmax-resource-bytes = 33\n[format]\nnewline = \"lf\"\n",
    ).expect("configuration fixture");
    let result = resolve_config(
        ProjectConfigRequest {
            authority: ProjectAuthority::open(
                directory.path().to_owned(),
                [directory.path().to_owned()],
            )
            .expect("authority"),
            search_from: directory.path().to_owned(),
            search_from_is_directory: true,
            config: ProjectConfigSelection::Discover,
            overrides: ProjectConfigOverrides::default(),
            limits: request_with(Vec::new()).limits,
        },
        &NeverCancel,
    )
    .expect("configuration resolves");
    assert_eq!(result.config.config.resource_limits().max_files, 7);
    assert!(result.config.config.format_newline_explicit());
    assert!(
        result
            .resources
            .iter()
            .any(|resource| resource.observation.is_some())
    );
}

#[test]
fn request_lint_overrides_apply_to_configuration_and_analysis() {
    let directory = tempfile::tempdir().expect("temporary directory");
    fs::write(
        directory.path().join("guide.adoc"),
        "本文xref:target.adoc[参照先]\n",
    )
    .expect("document fixture");
    let rule = adocweave_core::output::diagnostics::lint_rule("macro-boundary")
        .expect("known opt-in rule")
        .id;
    let mut request = ProjectRequest {
        targets: vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
        sources: Vec::new(),
        config: ProjectConfigSelection::Disabled,
        overrides: ProjectConfigOverrides {
            enable_lint_rules: vec![rule],
            ..ProjectConfigOverrides::default()
        },
        apply_safe_fixes: false,
        resource_selection: Default::default(),
        authority: ProjectAuthority::open(
            directory.path().to_owned(),
            [directory.path().to_owned()],
        )
        .expect("authority"),
        limits: request_with(Vec::new()).limits,
    };
    request.limits.max_output_bytes = u32::MAX;

    let result = process(request, &NeverCancel).expect("request applies lint overrides");
    let target = &result.targets[0];
    assert!(
        target
            .config
            .config
            .analysis()
            .diagnostics
            .lint
            .rule(rule)
            .enabled
    );
    assert!(
        target
            .analysis
            .as_ref()
            .expect("analysis")
            .primary
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "macro-boundary")
    );
}

#[test]
fn request_path_overrides_replace_resource_and_local_target_roots() {
    let project = tempfile::tempdir().expect("project directory");
    let external = tempfile::tempdir().expect("external directory");
    fs::write(
        project.path().join(".adocweave.toml"),
        "schema-version = 2\n[resources]\nroots = [\"configured\"]\n",
    )
    .expect("configuration fixture");
    let result = resolve_config(
        ProjectConfigRequest {
            authority: ProjectAuthority::open(
                project.path().to_owned(),
                [project.path().to_owned(), external.path().to_owned()],
            )
            .expect("authority"),
            search_from: project.path().to_owned(),
            search_from_is_directory: true,
            config: ProjectConfigSelection::Discover,
            overrides: ProjectConfigOverrides {
                resource_roots: Some(vec![external.path().to_owned()]),
                local_target_project_root: Some(external.path().to_owned()),
                ..ProjectConfigOverrides::default()
            },
            limits: request_with(Vec::new()).limits,
        },
        &NeverCancel,
    )
    .expect("configuration resolves with request path overrides");

    assert_eq!(
        result.config.config.resource_roots(),
        [external
            .path()
            .canonicalize()
            .expect("canonical external root")]
    );
    assert!(result.config.config.local_targets_enabled());
    assert_eq!(
        result.config.config.local_target_root(),
        Some(
            external
                .path()
                .canonicalize()
                .expect("canonical local-target root")
                .as_path()
        )
    );
}

#[test]
fn pathless_input_keeps_include_and_local_target_bases_separate() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let canonical_directory = directory
        .path()
        .canonicalize()
        .expect("canonical temporary directory");
    let include_base = directory.path().join("includes");
    let canonical_include_base = canonical_directory.join("includes");
    fs::create_dir(&include_base).expect("include directory");
    fs::write(include_base.join("part.adoc"), "included\n").expect("include fixture");
    fs::write(directory.path().join("asset.png"), "asset\n").expect("local target fixture");
    let id = SourceId::new("stdin");
    let mut request = ProjectRequest {
        targets: vec![ProjectTarget::Source(id.clone())],
        sources: vec![ProjectSource::memory(
            id,
            include_base.clone(),
            "include::part.adoc[]\n\nimage::asset.png[]\n",
        )],
        config: ProjectConfigSelection::Disabled,
        overrides: ProjectConfigOverrides {
            include: Some(true),
            local_target_project_root: Some(directory.path().to_owned()),
            ..ProjectConfigOverrides::default()
        },
        apply_safe_fixes: false,
        resource_selection: ProjectResourceSelection {
            local_targets: true,
            stylesheets: false,
        },
        authority: ProjectAuthority::open(
            directory.path().to_owned(),
            [directory.path().to_owned()],
        )
        .expect("authority"),
        limits: request_with(Vec::new()).limits,
    };
    request.limits.max_output_bytes = u32::MAX;

    let result = process(request, &NeverCancel).expect("pathless input is processed");
    let target = &result.targets[0];
    assert!(target.analysis.is_ok());
    let include = target
        .resources
        .iter()
        .find(|resource| {
            resource.kind == ProjectResourceKind::Include
                && resource.path == canonical_include_base.join("part.adoc")
        })
        .expect("include observation");
    assert_eq!(
        include
            .requested_at
            .as_ref()
            .map(|location| &location.source_id),
        Some(&SourceId::new("stdin"))
    );
    assert!(matches!(
        include.outcome,
        adocweave_project::ProjectResourceOutcome::Loaded { .. }
    ));
    let asset = target
        .resources
        .iter()
        .find(|resource| {
            resource.kind == ProjectResourceKind::LocalTarget
                && resource.path == canonical_directory.join("asset.png")
        })
        .expect("local-target observation");
    assert_eq!(
        asset
            .requested_at
            .as_ref()
            .map(|location| &location.source_id),
        Some(&SourceId::new("stdin"))
    );
    assert_eq!(
        asset.outcome,
        adocweave_project::ProjectResourceOutcome::Present
    );
    assert!(target.resources.iter().any(|resource| {
        resource.kind == ProjectResourceKind::LocalTarget
            && resource.path == canonical_include_base.join("part.adoc")
            && resource.outcome == adocweave_project::ProjectResourceOutcome::Present
    }));
    assert!(target.resources.iter().all(|resource| {
        resource.kind != ProjectResourceKind::LocalTarget
            || resource.outcome == adocweave_project::ProjectResourceOutcome::Present
    }));
}

#[test]
fn pathless_input_checks_same_relative_target_against_each_base() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let canonical_directory = directory
        .path()
        .canonicalize()
        .expect("canonical temporary directory");
    let include_base = directory.path().join("includes");
    let canonical_include_base = canonical_directory.join("includes");
    fs::create_dir(&include_base).expect("include directory");
    fs::write(include_base.join("same.adoc"), "included\n").expect("include fixture");

    for source in [
        "include::same.adoc[]\n\nimage::same.adoc[]\n",
        "image::same.adoc[]\n\ninclude::same.adoc[]\n",
    ] {
        let id = SourceId::new("stdin");
        let mut request = ProjectRequest {
            targets: vec![ProjectTarget::Source(id.clone())],
            sources: vec![ProjectSource::memory(id, include_base.clone(), source)],
            config: ProjectConfigSelection::Disabled,
            overrides: ProjectConfigOverrides {
                include: Some(true),
                local_target_project_root: Some(directory.path().to_owned()),
                ..ProjectConfigOverrides::default()
            },
            apply_safe_fixes: false,
            resource_selection: ProjectResourceSelection {
                local_targets: true,
                stylesheets: false,
            },
            authority: ProjectAuthority::open(
                directory.path().to_owned(),
                [directory.path().to_owned()],
            )
            .expect("authority"),
            limits: request_with(Vec::new()).limits,
        };
        request.limits.max_output_bytes = u32::MAX;

        let result = process(request, &NeverCancel).expect("pathless input is processed");
        let local_targets = result.targets[0]
            .resources
            .iter()
            .filter(|resource| resource.kind == ProjectResourceKind::LocalTarget)
            .collect::<Vec<_>>();
        assert!(local_targets.iter().any(|resource| {
            resource.path == canonical_include_base.join("same.adoc")
                && resource.outcome == adocweave_project::ProjectResourceOutcome::Present
                && resource
                    .requested_at
                    .as_ref()
                    .map(|location| &location.source_id)
                    == Some(&SourceId::new("stdin"))
        }));
        assert!(local_targets.iter().any(|resource| {
            resource.path == canonical_directory.join("same.adoc")
                && matches!(
                    resource.outcome,
                    adocweave_project::ProjectResourceOutcome::Missing
                )
                && resource
                    .requested_at
                    .as_ref()
                    .map(|location| &location.source_id)
                    == Some(&SourceId::new("stdin"))
        }));
    }
}

#[test]
fn pathless_bases_must_both_be_inside_the_request_authority() {
    let project = tempfile::tempdir().expect("project directory");
    let outside = tempfile::tempdir().expect("outside directory");
    let limits = request_with(Vec::new()).limits;
    let request = |base: PathBuf, local_root: PathBuf| {
        let id = SourceId::new("stdin");
        ProjectRequest {
            targets: vec![ProjectTarget::Source(id.clone())],
            sources: vec![ProjectSource::memory(id, base, "text\n")],
            config: ProjectConfigSelection::Disabled,
            overrides: ProjectConfigOverrides {
                local_target_project_root: Some(local_root),
                ..ProjectConfigOverrides::default()
            },
            apply_safe_fixes: false,
            resource_selection: Default::default(),
            authority: ProjectAuthority::open(
                project.path().to_owned(),
                [project.path().to_owned()],
            )
            .expect("project authority"),
            limits,
        }
    };

    assert!(matches!(
        process(
            request(outside.path().to_owned(), project.path().to_owned()),
            &NeverCancel
        ),
        Err(ProjectError::Authority(_))
    ));
    assert!(matches!(
        process(
            request(project.path().to_owned(), outside.path().to_owned()),
            &NeverCancel
        ),
        Err(ProjectError::Authority(_))
    ));
}

#[test]
fn pathless_input_does_not_replace_a_real_file_with_a_synthetic_name() {
    let directory = tempfile::tempdir().expect("temporary directory");
    fs::write(
        directory.path().join(".adocweave-memory-0.adoc"),
        "disk content\n",
    )
    .expect("disk include");
    let authority =
        ProjectAuthority::open(directory.path().to_owned(), [directory.path().to_owned()])
            .expect("authority");
    let id = SourceId::new("stdin");
    let mut request = ProjectRequest {
        targets: vec![ProjectTarget::Source(id.clone())],
        sources: vec![ProjectSource::memory(
            id,
            directory.path().to_owned(),
            "include::.adocweave-memory-0.adoc[]\n",
        )],
        config: ProjectConfigSelection::Disabled,
        overrides: ProjectConfigOverrides {
            include: Some(true),
            ..ProjectConfigOverrides::default()
        },
        apply_safe_fixes: false,
        resource_selection: Default::default(),
        authority,
        limits: request_with(Vec::new()).limits,
    };
    request.limits.max_output_bytes = u32::MAX;
    let result = process(request, &NeverCancel).expect("pathless input is processed");
    let target = &result.targets[0];
    assert_eq!(target.path, None);
    assert!(
        target
            .analysis
            .as_ref()
            .expect("analysis")
            .expanded
            .as_ref()
            .expect("include expansion succeeds")
            .preprocessed
            .document
            .source
            .contains("disk content")
    );
    assert!(target.resources.iter().all(|resource| {
        resource
            .observation
            .as_ref()
            .map(|value| value.path.as_path())
            != Some(directory.path().join("stdin.adoc").as_path())
    }));
}

#[test]
fn file_overlay_is_identified_as_input_and_is_not_watched() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("guide.adoc");
    fs::write(&path, "disk\n").expect("disk source");
    let id = SourceId::new("open-guide");
    let request = ProjectRequest {
        targets: vec![ProjectTarget::Source(id.clone())],
        sources: vec![ProjectSource::new(id, path, "overlay\n")],
        config: ProjectConfigSelection::Disabled,
        overrides: ProjectConfigOverrides::default(),
        apply_safe_fixes: false,
        resource_selection: Default::default(),
        authority: ProjectAuthority::open(
            directory.path().to_owned(),
            [directory.path().to_owned()],
        )
        .expect("authority"),
        limits: request_with(Vec::new()).limits,
    };
    let result = process(request, &NeverCancel).expect("overlay is processed");
    let primary = result.targets[0]
        .resources
        .iter()
        .find(|resource| {
            matches!(
                resource.kind,
                adocweave_project::ProjectResourceKind::Primary
            )
        })
        .expect("primary observation");
    assert_eq!(primary.origin, ProjectResourceOrigin::Input);
    assert_eq!(primary.observation, None);
}

#[test]
fn duplicate_and_unknown_source_ids_are_invalid_input() {
    let mut request = request_with(vec![ProjectTarget::Source(SourceId::new("missing"))]);
    request.config = ProjectConfigSelection::Disabled;
    assert!(matches!(
        process(request, &NeverCancel),
        Err(ProjectError::InvalidInput(_))
    ));

    let mut request = request_with(vec![ProjectTarget::Source(SourceId::new(
        "project:guide.adoc",
    ))]);
    request.sources.push(ProjectSource::memory(
        SourceId::new("project:guide.adoc"),
        request.authority.project_root().to_owned(),
        "text\n",
    ));
    request.config = ProjectConfigSelection::Disabled;
    assert!(matches!(
        process(request, &NeverCancel),
        Err(ProjectError::InvalidInput(_))
    ));
}

#[test]
fn caller_sources_cannot_use_generated_resource_ids() {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for id in ["local-target:caller", "include-request:caller:0:1"] {
        let mut request = request_with(vec![ProjectTarget::Source(SourceId::new(id))]);
        request.sources.push(ProjectSource::memory(
            SourceId::new(id),
            project_root.clone(),
            "text\n",
        ));

        assert!(matches!(
            process(request, &NeverCancel),
            Err(ProjectError::InvalidInput(error)) if error.code == "reserved-source-id"
        ));
    }
}

#[test]
fn file_overlay_is_reused_inside_a_narrow_include_authority() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let narrow = directory.path().join("narrow");
    fs::create_dir(&narrow).expect("narrow directory");
    fs::write(narrow.join("guide.adoc"), "include::part.adoc[]\n").expect("guide");
    fs::write(narrow.join("part.adoc"), "disk\n").expect("disk include");
    let overlay_id = SourceId::new("open-part");
    let mut request = ProjectRequest {
        targets: vec![ProjectTarget::Path(PathBuf::from("narrow/guide.adoc"))],
        sources: vec![ProjectSource::new(
            overlay_id.clone(),
            narrow.join("part.adoc"),
            "overlay\n",
        )],
        config: ProjectConfigSelection::Disabled,
        overrides: ProjectConfigOverrides::default(),
        apply_safe_fixes: false,
        resource_selection: Default::default(),
        authority: ProjectAuthority::open(
            directory.path().to_owned(),
            [directory.path().to_owned()],
        )
        .expect("authority"),
        limits: request_with(Vec::new()).limits,
    };
    request.overrides.include = Some(true);

    let result = process(request, &NeverCancel).expect("narrow include uses the overlay");
    let target = &result.targets[0];
    assert!(
        target
            .analysis
            .as_ref()
            .expect("analysis")
            .expanded
            .as_ref()
            .expect("include expansion succeeds")
            .preprocessed
            .document
            .source
            .contains("overlay")
    );
    assert!(target.resources.iter().any(|resource| {
        resource.kind == ProjectResourceKind::Include
            && resource.source_id == overlay_id
            && resource.origin == ProjectResourceOrigin::Input
            && resource.observation.is_none()
    }));
}

#[cfg(target_os = "linux")]
#[test]
fn project_authority_keeps_the_opened_child_identity() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let parent = directory.path();
    let project = parent.join("project");
    let old = parent.join("old-project");
    fs::create_dir(&project).expect("project directory");
    fs::write(project.join("guide.adoc"), "original\n").expect("original source");
    let authority =
        ProjectAuthority::open(project.clone(), [parent.to_owned()]).expect("authority");
    fs::rename(&project, &old).expect("move opened project");
    fs::create_dir(&project).expect("replacement project");
    fs::write(project.join("guide.adoc"), "replacement\n").expect("replacement source");
    let result = process(
        ProjectRequest {
            targets: vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
            sources: Vec::new(),
            config: ProjectConfigSelection::Disabled,
            overrides: ProjectConfigOverrides::default(),
            apply_safe_fixes: false,
            resource_selection: Default::default(),
            authority: authority.clone(),
            limits: request_with(Vec::new()).limits,
        },
        &NeverCancel,
    )
    .expect("request uses retained authority");
    assert_eq!(result.targets[0].source.as_deref(), Some("original\n"));

    let second = process(
        ProjectRequest {
            targets: vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))],
            sources: Vec::new(),
            config: ProjectConfigSelection::Disabled,
            overrides: ProjectConfigOverrides::default(),
            apply_safe_fixes: false,
            resource_selection: Default::default(),
            authority,
            limits: request_with(Vec::new()).limits,
        },
        &NeverCancel,
    )
    .expect("cloned authority retains the same opened identity");
    assert_eq!(second.targets[0].source.as_deref(), Some("original\n"));
}

#[cfg(target_os = "linux")]
#[test]
fn observer_keeps_the_opened_root_and_rejects_external_paths() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("project");
    let displaced = directory.path().join("opened-project");
    let outside = directory.path().join("outside");
    fs::create_dir(&root).expect("project root");
    fs::create_dir(&outside).expect("outside root");
    fs::write(root.join("guide.adoc"), "trusted\n").expect("trusted source");
    fs::write(outside.join("guide.adoc"), "outside\n").expect("outside source");
    let authority = ProjectAuthority::open(root.clone(), [root.clone()]).expect("authority");
    let access = authority.observation_access();
    fs::rename(&root, &displaced).expect("displace root");
    symlink(&outside, &root).expect("replacement root");

    let mut observer = access.observer();
    assert_eq!(
        observer.observe(
            &root.join("guide.adoc"),
            adocweave_project::ProjectObservationKind::Contents,
        ),
        adocweave_project::ProjectResourceObservation::from_bytes(b"trusted\n")
    );
    assert_eq!(
        observer.observe(
            &outside.join("guide.adoc"),
            adocweave_project::ProjectObservationKind::Contents,
        ),
        adocweave_project::ProjectResourceObservation::unavailable()
    );

    fs::remove_file(&root).expect("remove replacement");
    fs::rename(displaced, root).expect("restore root");
}

#[test]
fn observer_applies_one_shared_file_limit() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let target = directory.path().join("guide.adoc");
    fs::write(&target, "text\n").expect("source");
    let authority =
        ProjectAuthority::open(directory.path().to_owned(), [directory.path().to_owned()])
            .expect("authority");
    let mut observer = authority.observation_access().observer();
    for _ in 0..ProjectResourceLimits::default().max_files {
        assert_ne!(
            observer.observe(
                &target,
                adocweave_project::ProjectObservationKind::Existence,
            ),
            adocweave_project::ProjectResourceObservation::unavailable()
        );
    }
    assert_eq!(
        observer.observe(
            &target,
            adocweave_project::ProjectObservationKind::Existence,
        ),
        adocweave_project::ProjectResourceObservation::unavailable()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn write_capability_uses_the_opened_parent_after_root_replacement() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("project");
    let displaced = directory.path().join("opened-project");
    let outside = directory.path().join("outside");
    fs::create_dir(&root).expect("project root");
    fs::create_dir(&outside).expect("outside root");
    fs::write(root.join("guide.adoc"), "trusted\n").expect("trusted source");
    fs::write(outside.join("guide.adoc"), "outside\n").expect("outside source");
    let mut request = request_with(vec![ProjectTarget::Path(PathBuf::from("guide.adoc"))]);
    request.authority = ProjectAuthority::open(root.clone(), [root.clone()]).expect("authority");
    request.config = ProjectConfigSelection::Disabled;
    let result = process(request, &NeverCancel).expect("project result");
    let write = result
        .targets
        .into_iter()
        .next()
        .expect("target")
        .write
        .expect("write");

    fs::rename(&root, &displaced).expect("displace root");
    symlink(&outside, &root).expect("replacement root");
    assert!(
        write
            .replace_after_recheck(b"updated\n")
            .expect("safe replacement")
    );
    assert_eq!(
        fs::read_to_string(displaced.join("guide.adoc")).expect("opened target"),
        "updated\n"
    );
    assert_eq!(
        fs::read_to_string(outside.join("guide.adoc")).expect("outside target"),
        "outside\n"
    );

    fs::remove_file(&root).expect("remove replacement");
    fs::rename(displaced, root).expect("restore root");
}

#[test]
fn source_targets_are_deduplicated_and_sorted_by_source_id() {
    let mut request = request_with(Vec::new());
    request.config = ProjectConfigSelection::Disabled;
    for (id, source) in [("z", "z\n"), ("a", "a\n")] {
        request.sources.push(ProjectSource::memory(
            SourceId::new(id),
            request.authority.project_root().to_owned(),
            source,
        ));
    }
    request.targets = vec![
        ProjectTarget::Source(SourceId::new("z")),
        ProjectTarget::Source(SourceId::new("a")),
        ProjectTarget::Source(SourceId::new("z")),
    ];

    let result = process(request, &NeverCancel).expect("source targets are normalized");
    assert_eq!(result.targets.len(), 2);
    assert_eq!(result.targets[0].source_id.as_str(), "a");
    assert_eq!(result.targets[1].source_id.as_str(), "z");
}

#[test]
fn memory_sources_obey_request_file_resource_and_total_byte_limits() {
    let make_request = |sources: Vec<ProjectSource>, targets: Vec<ProjectTarget>| {
        let mut request = request_with(targets);
        request.config = ProjectConfigSelection::Disabled;
        request.sources = sources;
        request
    };

    let id = SourceId::new("large");
    let mut resource = make_request(
        vec![ProjectSource::memory(
            id.clone(),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            "five!",
        )],
        vec![ProjectTarget::Source(id)],
    );
    resource.limits.resources.max_resource_bytes = 4;
    assert!(matches!(
        process(resource, &NeverCancel),
        Err(ProjectError::Limit(ProjectLimit::ResourceBytes {
            limit: 4
        }))
    ));

    let ids = [SourceId::new("a"), SourceId::new("b")];
    let sources = ids
        .iter()
        .map(|id| {
            ProjectSource::memory(id.clone(), PathBuf::from(env!("CARGO_MANIFEST_DIR")), "abc")
        })
        .collect();
    let targets = ids.iter().cloned().map(ProjectTarget::Source).collect();
    let mut total = make_request(sources, targets);
    total.limits.resources.max_total_bytes = 5;
    assert!(matches!(
        process(total, &NeverCancel),
        Err(ProjectError::Limit(ProjectLimit::ReadBytes { limit: 5 }))
    ));

    let ids = [SourceId::new("c"), SourceId::new("d")];
    let sources = ids
        .iter()
        .map(|id| ProjectSource::memory(id.clone(), PathBuf::from(env!("CARGO_MANIFEST_DIR")), "x"))
        .collect();
    let targets = ids.iter().cloned().map(ProjectTarget::Source).collect();
    let mut files = make_request(sources, targets);
    files.limits.resources.max_files = 1;
    assert!(matches!(
        process(files, &NeverCancel),
        Err(ProjectError::Limit(ProjectLimit::Files { limit: 1 }))
    ));
}

#[test]
fn pathless_input_obeys_scope_and_output_limits() {
    let directory = tempfile::tempdir().expect("temporary directory");
    fs::write(
        directory.path().join(".adocweave.toml"),
        "schema-version = 2\n[resources]\nmax-files = 1\nmax-total-bytes = 1024\nmax-resource-bytes = 4\n",
    )
    .expect("configuration");
    let id = SourceId::new("stdin");
    let mut request = ProjectRequest {
        targets: vec![ProjectTarget::Source(id.clone())],
        sources: vec![ProjectSource::memory(
            id,
            directory.path().to_owned(),
            "too large\n",
        )],
        config: ProjectConfigSelection::Discover,
        overrides: ProjectConfigOverrides::default(),
        apply_safe_fixes: false,
        resource_selection: Default::default(),
        authority: ProjectAuthority::open(
            directory.path().to_owned(),
            [directory.path().to_owned()],
        )
        .expect("authority"),
        limits: request_with(Vec::new()).limits,
    };
    let result = process(request, &NeverCancel).expect("scope failure is target-local");
    assert!(matches!(
        result.targets[0].analysis,
        Err(adocweave_project::ProjectTargetError::Incomplete(_))
    ));
    assert_eq!(result.targets[0].source, None);

    request = request_with(Vec::new());
    let id = SourceId::new("small-output");
    request.sources.push(ProjectSource::memory(
        id.clone(),
        request.authority.project_root().to_owned(),
        "output text\n",
    ));
    request.targets.push(ProjectTarget::Source(id));
    request.config = ProjectConfigSelection::Disabled;
    request.limits.max_output_bytes = 1;
    let result = process(request, &NeverCancel).expect("output failure is target-local");
    assert_eq!(result.targets[0].source, None);
}

#[test]
fn memory_sources_cannot_replace_configuration_or_stylesheets() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let config = directory.path().join(".adocweave.toml");
    let stylesheet = directory.path().join("style.css");
    let guide = directory.path().join("guide.adoc");
    fs::write(
        &config,
        "schema-version = 2\n[html]\nstylesheet-files = [\"style.css\"]\n",
    )
    .expect("configuration");
    fs::write(&stylesheet, "disk stylesheet\n").expect("stylesheet");
    fs::write(&guide, "text\n").expect("guide");
    let request = ProjectRequest {
        targets: vec![ProjectTarget::Path(guide)],
        sources: vec![
            ProjectSource::new(SourceId::new("config-overlay"), config, "invalid"),
            ProjectSource::new(
                SourceId::new("stylesheet-overlay"),
                stylesheet,
                "memory stylesheet\n",
            ),
        ],
        config: ProjectConfigSelection::Discover,
        overrides: ProjectConfigOverrides::default(),
        apply_safe_fixes: false,
        resource_selection: ProjectResourceSelection {
            local_targets: false,
            stylesheets: true,
        },
        authority: ProjectAuthority::open(
            directory.path().to_owned(),
            [directory.path().to_owned()],
        )
        .expect("authority"),
        limits: request_with(Vec::new()).limits,
    };
    let result = process(request, &NeverCancel).expect("disk configuration remains authoritative");
    let stylesheet = result.targets[0]
        .resources
        .iter()
        .find(|resource| resource.kind == adocweave_project::ProjectResourceKind::Stylesheet)
        .expect("stylesheet observation");
    assert_eq!(stylesheet.origin, ProjectResourceOrigin::Filesystem);
    assert!(
        matches!(&stylesheet.outcome, adocweave_project::ProjectResourceOutcome::Loaded { source } if source.contains("disk stylesheet"))
    );
}

struct CancelAfter {
    calls: AtomicUsize,
    limit: usize,
}

impl adocweave_core::CancellationCheck for CancelAfter {
    fn is_cancelled(&self) -> bool {
        self.calls.fetch_add(1, Ordering::Relaxed) >= self.limit
    }
}

#[test]
fn config_only_cancellation_is_request_wide() {
    let directory = tempfile::tempdir().expect("temporary directory");
    fs::create_dir(directory.path().join("a")).expect("nested directory");
    let result = resolve_config(
        ProjectConfigRequest {
            authority: ProjectAuthority::open(
                directory.path().to_owned(),
                [directory.path().to_owned()],
            )
            .expect("authority"),
            search_from: directory.path().join("a"),
            search_from_is_directory: true,
            config: ProjectConfigSelection::Discover,
            overrides: ProjectConfigOverrides::default(),
            limits: request_with(Vec::new()).limits,
        },
        &CancelAfter {
            calls: AtomicUsize::new(0),
            limit: 1,
        },
    );
    assert!(matches!(result, Err(ProjectError::Cancelled)));
}

#[test]
fn directory_scan_observes_cancellation_during_the_walk() {
    let directory = tempfile::tempdir().expect("temporary directory");
    for index in 0..32 {
        let child = directory.path().join(format!("directory-{index}"));
        fs::create_dir(&child).expect("scan directory");
        fs::write(child.join("guide.adoc"), "text\n").expect("scan source");
    }
    let request = ProjectRequest {
        targets: vec![ProjectTarget::Directory(directory.path().to_owned())],
        sources: Vec::new(),
        config: ProjectConfigSelection::Disabled,
        overrides: ProjectConfigOverrides::default(),
        apply_safe_fixes: false,
        resource_selection: Default::default(),
        authority: ProjectAuthority::open(
            directory.path().to_owned(),
            [directory.path().to_owned()],
        )
        .expect("authority"),
        limits: request_with(Vec::new()).limits,
    };
    let result = process(
        request,
        &CancelAfter {
            calls: AtomicUsize::new(0),
            limit: 3,
        },
    );
    assert!(matches!(result, Err(ProjectError::Cancelled)));
}

#[test]
fn document_cancellation_is_always_request_wide() {
    let directory = tempfile::tempdir().expect("temporary directory");
    fs::write(directory.path().join("part.adoc"), "included\n").expect("include fixture");
    fs::write(
        directory.path().join("guide.adoc"),
        format!("{}include::part.adoc[]\n", "paragraph\n\n".repeat(128)),
    )
    .expect("primary fixture");
    let mut cancelled = 0_usize;
    let mut completed = 0_usize;
    for limit in (1..64).chain([usize::MAX]) {
        let mut request = request_with(vec![ProjectTarget::Path(
            directory.path().join("guide.adoc"),
        )]);
        request.authority =
            ProjectAuthority::open(directory.path().to_owned(), [directory.path().to_owned()])
                .expect("authority");
        request.config = ProjectConfigSelection::Disabled;
        request.overrides.include = Some(true);
        match process(
            request,
            &CancelAfter {
                calls: AtomicUsize::new(0),
                limit,
            },
        ) {
            Err(ProjectError::Cancelled) => cancelled += 1,
            Ok(_) => {
                completed += 1;
            }
            Err(error) => panic!("unexpected project error: {error}"),
        }
    }
    assert!(cancelled > 0, "at least one checkpoint cancels the request");
    assert!(
        completed > 0,
        "a later checkpoint allows the request to finish"
    );
}
