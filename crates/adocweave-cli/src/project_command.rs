use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::{self, Read as _};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use adocweave::NeverCancel;
use adocweave_project::{
    ConfigSelection, ProjectAuthority, ProjectConfigRequest, ProjectLimits, ProjectOverrides,
    ProjectRequest, ProjectResourceKind, ProjectResourceOutcome, ProjectResourceSelection,
    ProjectSource, ProjectTarget, ProjectTargetResult, process, resolve_config,
};

use crate::arguments::{Arguments, CommandOptions};
use crate::check_output::{DiagnosticCounts, DiagnosticFormat, sarif_log, sarif_results};
use crate::cli_error::{CliError, check_error, convert_error, format_error};
use crate::commands;
use crate::file_workflow::{PendingWrite, WriteOutcome, apply_file_writes, colorize_lines};
use crate::finish_output;

pub(crate) fn run(arguments: &Arguments) -> Result<ExitCode, CliError> {
    let current = env::current_dir().map_err(|source| CliError::Read {
        source_name: "current directory".to_owned(),
        source,
    })?;
    let current = current.canonicalize().map_err(|source| CliError::Read {
        source_name: "current directory".to_owned(),
        source,
    })?;
    if matches!(arguments.command, CommandOptions::ConfigShow) {
        return show_config(arguments, &current);
    }
    let request = request(arguments, &current)?;
    let mut result = process(request, &NeverCancel).map_err(CliError::Project)?;
    if result.targets.is_empty() {
        return Err(CliError::Path(
            "no AsciiDoc files matched the input paths".to_owned(),
        ));
    }
    if !arguments.allowed_roots.is_empty()
        && result
            .targets
            .iter()
            .any(|target| !target.config.config.include_enabled())
    {
        return Err(CliError::Usage(
            "--allow-root requires include processing".to_owned(),
        ));
    }
    match &arguments.command {
        CommandOptions::Convert { complete, css } => {
            let target = only_target(&result.targets)?;
            let analysis = expanded_analysis(target)?;
            let policy = commands::html_policy::build_project(
                &target.config.config,
                &target.resources,
                *complete,
                css,
            )
            .map_err(crate::cli_error::html_policy_error)?;
            let output =
                commands::convert::render_analysis(&analysis.preprocessed.analysis, &policy)
                    .map_err(convert_error)?;
            print_output(finish_output(output)?)?;
            Ok(ExitCode::SUCCESS)
        }
        CommandOptions::Symbols => {
            let target = only_target(&result.targets)?;
            let analysis = expanded_analysis(target)?;
            let output = commands::symbols::render_analysis(&analysis.preprocessed.analysis);
            print_output(finish_output(output)?)?;
            Ok(ExitCode::SUCCESS)
        }
        CommandOptions::Format(options) => run_format(arguments, &mut result.targets, *options),
        CommandOptions::Check(options) => {
            run_check(arguments, &current, &mut result.targets, options)
        }
        CommandOptions::ConfigShow | CommandOptions::Preview { .. } => {
            unreachable!("handled outside project one-shot processing")
        }
    }
}

fn show_config(arguments: &Arguments, current: &Path) -> Result<ExitCode, CliError> {
    let authority = project_authority(arguments, current)?;
    let result = resolve_config(
        ProjectConfigRequest {
            authority,
            search_from: current.to_owned(),
            search_from_is_directory: true,
            config: config_selection(arguments),
            overrides: ProjectOverrides::default(),
            limits: ProjectLimits::default(),
        },
        &NeverCancel,
    )
    .map_err(CliError::Project)?;
    println!("{}", commands::config::run_project(&result.config).output);
    Ok(ExitCode::SUCCESS)
}

fn request(arguments: &Arguments, current: &Path) -> Result<ProjectRequest, CliError> {
    let authority = project_authority(arguments, current)?;
    request_with_authority(arguments, current, authority)
}

pub(crate) fn request_with_authority(
    arguments: &Arguments,
    current: &Path,
    authority: ProjectAuthority,
) -> Result<ProjectRequest, CliError> {
    if arguments.no_include
        && (arguments.stdin_base.is_some() || !arguments.allowed_roots.is_empty())
    {
        return Err(CliError::Usage(
            "--stdin-base and --allow-root require include processing".to_owned(),
        ));
    }
    let limits = ProjectLimits::default();
    let mut targets = Vec::new();
    let mut sources = Vec::new();
    if let Some(input) = &arguments.input {
        for path in std::iter::once(input).chain(&arguments.additional_inputs) {
            targets.push(path_target(current, path));
        }
        targets.extend(
            arguments
                .glob_patterns
                .iter()
                .cloned()
                .map(ProjectTarget::Glob),
        );
    } else {
        if arguments.include && arguments.stdin_base.is_none() {
            return Err(CliError::Usage(
                "--include with standard input requires --stdin-base".to_owned(),
            ));
        }
        if !arguments.allowed_roots.is_empty() && arguments.stdin_base.is_none() {
            return Err(CliError::Usage(
                "--stdin-base and --allow-root require include processing".to_owned(),
            ));
        }
        let bytes = read_standard_input(limits)?;
        let source = String::from_utf8(bytes).map_err(|error| CliError::InvalidUtf8 {
            valid_up_to: error.utf8_error().valid_up_to(),
        })?;
        let source_id = adocweave::SourceId::new("<stdin>");
        let base = arguments.stdin_base.as_ref().map_or_else(
            || current.to_owned(),
            |path| absolute_lexical(current, path),
        );
        sources.push(ProjectSource::memory(source_id.clone(), base, source));
        targets.push(ProjectTarget::Source(source_id));
    }
    let include = if arguments.no_include {
        Some(false)
    } else if arguments.include {
        Some(true)
    } else if arguments.input.is_none() && arguments.stdin_base.is_none() {
        Some(false)
    } else {
        None
    };
    let stylesheet_files = match &arguments.command {
        CommandOptions::Convert { css, .. } | CommandOptions::Preview { css, .. } => css
            .iter()
            .filter_map(|value| match value {
                commands::html_policy::StylesheetArgument::File(path) => {
                    Some(absolute_lexical(current, path))
                }
                commands::html_policy::StylesheetArgument::Url(_) => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let enabled_rules = match &arguments.command {
        CommandOptions::Check(options) => options.enabled_rules.clone(),
        _ => Vec::new(),
    };
    let resource_selection = ProjectResourceSelection {
        local_targets: matches!(
            arguments.command,
            CommandOptions::Check(_) | CommandOptions::Preview { .. }
        ),
        stylesheets: matches!(
            arguments.command,
            CommandOptions::Convert { .. } | CommandOptions::Preview { .. }
        ),
    };
    Ok(ProjectRequest {
        targets,
        sources,
        config: config_selection(arguments),
        overrides: ProjectOverrides {
            include,
            enable_lint_rules: enabled_rules,
            resource_roots: (!arguments.allowed_roots.is_empty()).then(|| {
                arguments
                    .allowed_roots
                    .iter()
                    .map(|path| absolute_lexical(current, path))
                    .collect()
            }),
            local_target_project_root: arguments
                .project_root
                .as_ref()
                .map(|path| absolute_lexical(current, path)),
            stylesheet_files,
        },
        apply_safe_fixes: matches!(
            arguments.command,
            CommandOptions::Check(commands::check::Options { fix: true, .. })
        ),
        resource_selection,
        authority,
        limits,
    })
}

fn run_format(
    arguments: &Arguments,
    targets: &mut [ProjectTargetResult],
    options: commands::format::Options,
) -> Result<ExitCode, CliError> {
    if options.write && targets.iter().any(|target| target.path.is_none()) {
        return Err(CliError::Usage(
            "format --write requires file inputs".to_owned(),
        ));
    }
    if targets.len() > 1 && !options.supports_multiple_inputs() {
        return Err(CliError::Usage(
            "multiple format inputs require --check, --write, or --diff".to_owned(),
        ));
    }
    if targets.len() == 1
        && (targets[0].path.is_none() || (!options.check && !options.write && !options.diff))
    {
        let target = &targets[0];
        let _ = expanded_analysis(target)?;
        let source = target
            .source
            .as_deref()
            .ok_or_else(|| target_error(target))?;
        let analysis = target
            .analysis
            .as_ref()
            .map_err(|error| target_cli_error(target, error))?;
        let config =
            commands::format::project_format_config(options, source, &target.config.config);
        let formatted =
            commands::format::process_analysis(&analysis.primary, &config).map_err(format_error)?;
        if options.check && formatted != source {
            return Err(CliError::FormattingRequired);
        }
        if !options.check {
            print_output(finish_output(formatted)?)?;
        }
        return Ok(ExitCode::SUCCESS);
    }
    let mut workflow = commands::format::BatchWorkflow::new(options, targets.len());
    for target in targets.iter_mut() {
        let _ = expanded_analysis(target)?;
        let path = target.path.as_ref().ok_or_else(|| target_error(target))?;
        let source = target
            .source
            .as_deref()
            .ok_or_else(|| target_error(target))?;
        let analysis = target
            .analysis
            .as_ref()
            .map_err(|error| target_cli_error(target, error))?;
        let config =
            commands::format::project_format_config(options, source, &target.config.config);
        let formatted =
            commands::format::process_analysis(&analysis.primary, &config).map_err(format_error)?;
        workflow
            .record(
                path.clone(),
                source.as_bytes().to_vec(),
                formatted.into_bytes(),
            )
            .map_err(format_error)?;
    }
    let outcome = workflow.finish();
    let legacy_summary = outcome.summary();
    let unchanged = outcome.files.saturating_sub(outcome.changed);
    let write_outcome = if outcome.pending_writes.is_empty() {
        WriteOutcome::default()
    } else {
        apply_file_writes(
            outcome
                .pending_writes
                .into_iter()
                .map(|write| {
                    let capability = targets
                        .iter_mut()
                        .find(|target| target.path.as_ref() == Some(&write.path))
                        .and_then(|target| target.write.take())
                        .ok_or_else(|| {
                            CliError::Path(format!(
                                "write authority is unavailable for {}",
                                write.path.display()
                            ))
                        })?;
                    pending_write(write.path, write.replacement, capability)
                })
                .collect::<Result<Vec<_>, _>>()?,
        )
    };
    report_write_failures(&write_outcome);
    let updated = write_outcome.updated;
    let failed = write_outcome.failures.len();
    if !outcome.output.is_empty() {
        print_output(finish_output(colorize_lines(
            &outcome.output,
            arguments.color,
        ))?)?;
    }
    if options.summary {
        if options.write {
            eprintln!(
                "adocweave format: files={}, changed={}, updated={updated}, unchanged={unchanged}, failed={failed}",
                outcome.files, outcome.changed
            );
        } else {
            eprintln!("{legacy_summary}");
        }
    }
    if failed > 0 {
        return Err(partial_write_error(
            outcome.files,
            outcome.changed,
            updated,
            unchanged,
            failed,
        ));
    }
    Ok(if outcome.formatting_required {
        adocweave_host::ExitStatus::Diagnostics.into()
    } else {
        ExitCode::SUCCESS
    })
}

fn run_check(
    arguments: &Arguments,
    current: &Path,
    targets: &mut [ProjectTargetResult],
    options: &commands::check::Options,
) -> Result<ExitCode, CliError> {
    if options.fix && !options.diff && targets.iter().any(|target| target.path.is_none()) {
        return Err(CliError::Usage(
            "check --fix requires file inputs".to_owned(),
        ));
    }
    let mut output = String::new();
    let mut machine_results = Vec::new();
    let mut counts = DiagnosticCounts::default();
    let mut writes = Vec::new();
    let mut changed = 0_usize;
    for target in targets.iter_mut() {
        let original = target
            .source
            .as_deref()
            .ok_or_else(|| target_error(target))?;
        if let Some(replacement) = target.replacement_source.as_deref() {
            changed += 1;
            let path = target
                .path
                .as_ref()
                .ok_or_else(|| CliError::Usage("check --fix requires file inputs".to_owned()))?;
            if options.diff {
                output.push_str(&commands::format::unified_diff(path, original, replacement));
            } else {
                writes.push(pending_write(
                    path.clone(),
                    replacement.as_bytes().to_vec(),
                    target.write.take().ok_or_else(|| {
                        CliError::Path(format!(
                            "write authority is unavailable for {}",
                            path.display()
                        ))
                    })?,
                )?);
            }
        }
        let sources = check_sources(target, current)?;
        let analysis = expanded_analysis(target)?;
        let checked =
            commands::check::process_project(analysis, options, &sources).map_err(check_error)?;
        counts.merge(checked.counts);
        if options.format == DiagnosticFormat::Json {
            machine_results.extend(
                serde_json::from_str::<Vec<serde_json::Value>>(&checked.output)
                    .map_err(|error| CliError::Serialize(error.to_string()))?,
            );
        } else if options.format == DiagnosticFormat::Sarif {
            machine_results.extend(sarif_results(&checked.output));
        } else {
            output.push_str(&checked.output);
        }
    }
    let unchanged = targets.len().saturating_sub(changed);
    let write_outcome = apply_file_writes(writes);
    report_write_failures(&write_outcome);
    let updated = write_outcome.updated;
    let failed = write_outcome.failures.len();
    if options.format == DiagnosticFormat::Json {
        output = serde_json::to_string(&machine_results)
            .map_err(|error| CliError::Serialize(error.to_string()))?;
    } else if options.format == DiagnosticFormat::Sarif {
        output = sarif_log(machine_results);
    }
    if options.format == DiagnosticFormat::Human {
        output = colorize_lines(&output, arguments.color);
    }
    print_output(finish_output(output)?)?;
    if options.summary {
        if options.fix && !options.diff {
            eprintln!(
                "adocweave check: {}, files={}, changed={changed}, updated={updated}, unchanged={unchanged}, failed={failed}",
                counts.summary(),
                targets.len()
            );
        } else if options.fix {
            eprintln!("adocweave check: {}, changed={changed}", counts.summary());
        } else {
            eprintln!("adocweave check: {}", counts.summary());
        }
    }
    if failed > 0 {
        return Err(partial_write_error(
            targets.len(),
            changed,
            updated,
            unchanged,
            failed,
        ));
    }
    Ok(if counts.fails(options.fail_on) {
        adocweave_host::ExitStatus::Diagnostics.into()
    } else {
        ExitCode::SUCCESS
    })
}

fn check_sources<'target>(
    target: &'target ProjectTargetResult,
    current: &Path,
) -> Result<BTreeMap<adocweave::SourceId, commands::check::ProjectSourceView<'target>>, CliError> {
    let analysis = expanded_analysis(target)?;
    let mut displays = BTreeMap::new();
    displays.insert(
        target.source_id.clone(),
        target
            .path
            .as_deref()
            .map_or_else(|| "<stdin>".to_owned(), |path| display_path(path, current)),
    );
    for directive in &analysis.source_mapping.directives {
        if let Some(source_id) = &directive.resource_source_id {
            displays.entry(source_id.clone()).or_insert_with(|| {
                let target = directive
                    .target
                    .strip_prefix("__adocweave_base__/")
                    .unwrap_or(&directive.target);
                format!("include:{target}")
            });
        }
    }
    let mut sources = BTreeMap::new();
    let primary = target
        .replacement_source
        .as_deref()
        .or(target.source.as_deref())
        .ok_or_else(|| target_error(target))?;
    sources.insert(
        target.source_id.clone(),
        commands::check::ProjectSourceView {
            display_id: displays
                .get(&target.source_id)
                .cloned()
                .unwrap_or_else(|| target.source_id.as_str().to_owned()),
            source: primary,
        },
    );
    for resource in &target.resources {
        if resource.kind != ProjectResourceKind::Include {
            continue;
        }
        let ProjectResourceOutcome::Loaded { source } = &resource.outcome else {
            continue;
        };
        sources.insert(
            resource.source_id.clone(),
            commands::check::ProjectSourceView {
                display_id: displays
                    .get(&resource.source_id)
                    .cloned()
                    .unwrap_or_else(|| resource.source_id.as_str().to_owned()),
                source,
            },
        );
    }
    Ok(sources)
}

pub(crate) fn project_authority(
    arguments: &Arguments,
    current: &Path,
) -> Result<ProjectAuthority, CliError> {
    ProjectAuthority::open(current.to_owned(), authority_roots(arguments, current))
        .map_err(|error| CliError::Project(adocweave_project::ProjectError::Authority(error)))
}

pub(crate) fn authority_roots(arguments: &Arguments, current: &Path) -> BTreeSet<PathBuf> {
    let mut roots = BTreeSet::from([current.to_owned()]);
    let mut add_directory = |path: &Path| {
        let path = absolute_lexical(current, path);
        if !path.starts_with(current) {
            roots.insert(path);
        }
    };
    for path in arguments
        .allowed_roots
        .iter()
        .chain(arguments.project_root.iter())
        .chain(arguments.stdin_base.iter())
    {
        add_directory(path);
    }
    if let Some(config) = &arguments.config_path {
        let path = absolute_lexical(current, config);
        add_directory(path.parent().unwrap_or(current));
    }
    for path in arguments.input.iter().chain(&arguments.additional_inputs) {
        let path = absolute_lexical(current, path);
        let directory = fs::symlink_metadata(&path)
            .ok()
            .filter(|metadata| metadata.is_dir())
            .map_or_else(|| path.parent().unwrap_or(current), |_| path.as_path());
        add_directory(directory);
    }
    if let CommandOptions::Convert { css, .. } | CommandOptions::Preview { css, .. } =
        &arguments.command
    {
        for path in css.iter().filter_map(|value| match value {
            commands::html_policy::StylesheetArgument::File(path) => Some(path),
            commands::html_policy::StylesheetArgument::Url(_) => None,
        }) {
            let path = absolute_lexical(current, path);
            add_directory(path.parent().unwrap_or(current));
        }
    }
    roots
}

fn path_target(current: &Path, path: &Path) -> ProjectTarget {
    let absolute = absolute_lexical(current, path);
    if fs::symlink_metadata(&absolute).is_ok_and(|metadata| metadata.is_dir()) {
        ProjectTarget::Directory(path.to_owned())
    } else {
        ProjectTarget::Path(path.to_owned())
    }
}

fn config_selection(arguments: &Arguments) -> ConfigSelection {
    if arguments.no_config {
        ConfigSelection::Disabled
    } else if let Some(path) = &arguments.config_path {
        ConfigSelection::Explicit(path.clone())
    } else {
        ConfigSelection::Discover
    }
}

fn absolute_lexical(root: &Path, path: &Path) -> PathBuf {
    let input = if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in input.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
        }
    }
    normalized
}

fn read_standard_input(limits: ProjectLimits) -> Result<Vec<u8>, CliError> {
    let limit = limits.max_resource_bytes.min(limits.max_read_bytes);
    let mut input = Vec::new();
    io::stdin()
        .take(limit.saturating_add(1))
        .read_to_end(&mut input)
        .map_err(|source| CliError::Read {
            source_name: "standard input".to_owned(),
            source,
        })?;
    let bytes = u64::try_from(input.len()).unwrap_or(u64::MAX);
    if bytes > limits.max_resource_bytes {
        return Err(CliError::Project(adocweave_project::ProjectError::Limit(
            adocweave_project::ProjectLimit::ResourceBytes {
                limit: limits.max_resource_bytes,
            },
        )));
    }
    if bytes > limits.max_read_bytes {
        return Err(CliError::Project(adocweave_project::ProjectError::Limit(
            adocweave_project::ProjectLimit::ReadBytes {
                limit: limits.max_read_bytes,
            },
        )));
    }
    Ok(input)
}

fn pending_write(
    path: PathBuf,
    replacement: Vec<u8>,
    capability: adocweave_project::ProjectWriteCapability,
) -> Result<PendingWrite, CliError> {
    if capability.path() != path {
        return Err(CliError::Path(
            "write authority does not match its project target".to_owned(),
        ));
    }
    Ok(PendingWrite {
        path,
        replacement,
        capability,
    })
}

fn report_write_failures(outcome: &WriteOutcome) {
    for failure in &outcome.failures {
        eprintln!(
            "adocweave: failed to update {}: {}",
            failure.path.display(),
            failure.message
        );
    }
}

fn partial_write_error(
    files: usize,
    changed: usize,
    updated: usize,
    unchanged: usize,
    failed: usize,
) -> CliError {
    CliError::PartialWrite {
        files,
        changed,
        updated,
        unchanged,
        failed,
    }
}

fn target_error(target: &ProjectTargetResult) -> CliError {
    match &target.analysis {
        Err(error) => target_cli_error(target, error),
        Ok(analysis) => analysis.expanded.as_ref().map_or_else(
            |error| CliError::ProjectExpansion(error.clone()),
            |_| CliError::Path("project result does not contain the primary source".to_owned()),
        ),
    }
}

fn expanded_analysis(
    target: &ProjectTargetResult,
) -> Result<&adocweave_project::ProjectExpandedAnalysis, CliError> {
    target
        .analysis
        .as_ref()
        .map_err(|error| target_cli_error(target, error))?
        .expanded
        .as_ref()
        .map_err(|error| CliError::ProjectExpansion(error.clone()))
}

fn target_cli_error(
    target: &ProjectTargetResult,
    error: &adocweave_project::ProjectTargetError,
) -> CliError {
    let primary = match error {
        adocweave_project::ProjectTargetError::Read(source) => {
            source.path.as_deref() == target.path.as_deref()
        }
        adocweave_project::ProjectTargetError::Incomplete(limit) => {
            target.resources.iter().any(|resource| {
                resource.kind == ProjectResourceKind::Primary
                    && matches!(
                        &resource.outcome,
                        ProjectResourceOutcome::Failed(
                            adocweave_project::ProjectResourceFailure::Limit(resource_limit)
                        ) if resource_limit == limit
                    )
            })
        }
        _ => false,
    };
    if primary {
        CliError::ProjectPrimary(error.clone())
    } else {
        CliError::ProjectTarget(error.clone())
    }
}

fn only_target(targets: &[ProjectTargetResult]) -> Result<&ProjectTargetResult, CliError> {
    if targets.len() == 1 {
        Ok(&targets[0])
    } else {
        Err(CliError::Usage(
            "this command requires exactly one input".to_owned(),
        ))
    }
}

fn display_path(path: &Path, current: &Path) -> String {
    path.strip_prefix(current)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn print_output(output: String) -> Result<(), CliError> {
    use std::io::Write as _;
    io::stdout()
        .write_all(output.as_bytes())
        .map_err(CliError::Write)
}
