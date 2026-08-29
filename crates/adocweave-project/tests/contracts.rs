use std::path::PathBuf;

use adocweave::OutputLimits;
use adocweave_config::ResolvedProjectConfig;
use adocweave_host::{FilesystemReadLimits, LocalFilesystemPolicy, LogicalSourceId, ResourceError};
use adocweave_project::{
    ConfigSelection, ProjectError, ProjectLimit, ProjectLimits, ProjectOutcome, ProjectOverrides,
    ProjectRequest, ProjectResult, ProjectTarget, ProjectTargetError, ProjectTargetResult,
    ProjectUsage, ProjectWarning,
};

fn request_with(targets: Vec<ProjectTarget>) -> ProjectRequest {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let authority =
        LocalFilesystemPolicy::new([project_root.clone()], FilesystemReadLimits::default())
            .expect("the crate directory is an existing filesystem authority");
    ProjectRequest {
        project_root,
        targets,
        config: ConfigSelection::Discover,
        overrides: ProjectOverrides::default(),
        authority,
        limits: ProjectLimits {
            filesystem_reads: FilesystemReadLimits::default(),
            max_directory_entries: 100_000,
            max_processing_iterations: 10_000,
            output: OutputLimits::default(),
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
fn request_wide_and_target_failures_are_distinct() {
    let config_error = ResolvedProjectConfig::parse("[", std::path::Path::new("."))
        .expect_err("invalid TOML must be rejected");
    let request_wide: ProjectOutcome = Err(ProjectError::Config(config_error));
    let target_error =
        ProjectTargetError::Read(ResourceError::Missing(PathBuf::from("missing.adoc")));

    assert!(matches!(request_wide, Err(ProjectError::Config(_))));
    assert!(matches!(target_error, ProjectTargetError::Read(_)));
}

#[test]
fn partial_scan_warning_preserves_collected_targets() {
    let target = ProjectTargetResult {
        source_id: LogicalSourceId::new("guide.adoc").expect("valid logical source ID"),
        path: PathBuf::from("guide.adoc"),
        config: None,
        resolved_config: ResolvedProjectConfig::default(),
        resources: Vec::new(),
        outcome: Err(ProjectTargetError::Incomplete(
            ProjectLimit::ProcessingIterations { limit: 8 },
        )),
    };
    let result = ProjectResult {
        targets: vec![target],
        warnings: vec![ProjectWarning::ScanTruncated { limit: 100 }],
        usage: ProjectUsage::default(),
    };

    assert_eq!(result.targets.len(), 1);
    assert_eq!(
        result.warnings,
        [ProjectWarning::ScanTruncated { limit: 100 }]
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
