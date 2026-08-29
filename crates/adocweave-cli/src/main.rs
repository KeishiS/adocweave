use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use adocweave::OutputLimits;
use adocweave::output::diagnostics as diagnostic;

mod arguments;
mod check_output;
mod cli_error;
mod commands;
mod diagnostic_json;
mod diagnostic_output;
mod file_workflow;
mod local_include;
mod local_target;
mod preview;
mod project_command;

static PREVIEW_SHUTDOWN: AtomicBool = AtomicBool::new(false);
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(unix)]
fn install_preview_signal_handlers() {
    extern "C" fn shutdown(_: libc::c_int) {
        PREVIEW_SHUTDOWN.store(true, std::sync::atomic::Ordering::Release);
    }
    // SAFETY: the handler performs only a lock-free atomic store, and the
    // process retains the static flag for its entire lifetime.
    unsafe {
        let handler = shutdown as *const () as libc::sighandler_t;
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }
}

#[cfg(not(unix))]
fn install_preview_signal_handlers() {}

#[cfg(test)]
use commands::format::Options as FormatOptions;
const DEFAULT_PREVIEW_PORT: u16 = 4000;
const DEFAULT_PREVIEW_DEBOUNCE_MS: u64 = 100;

use adocweave_host::ExitStatus;

use arguments::{Action, Arguments, CommandOptions, CompletionShell, command, parse_arguments};
use cli_error::{CliError, preview_error};

fn filesystem_authority(
    boundary: PathBuf,
) -> Result<adocweave_host::LocalFilesystemPolicy, CliError> {
    adocweave_host::LocalFilesystemPolicy::new(
        [boundary],
        adocweave_host::FilesystemReadLimits::default(),
    )
    .map_err(local_include::LocalIncludeError::Host)
    .map_err(CliError::Include)
}

fn resolve_primary_path(
    path: &Path,
    boundary_policy: &adocweave_host::LocalTargetPolicy,
) -> PathBuf {
    let candidate = if path.is_absolute() {
        path.to_owned()
    } else {
        boundary_policy.root().join(path)
    };
    match boundary_policy.normalize_candidate(&candidate) {
        Ok(candidate) => boundary_policy
            .inspect_candidate(&candidate)
            .unwrap_or(candidate),
        Err(adocweave_host::LocalTargetError::OutsideRoot(_)) => {
            path.canonicalize().unwrap_or_else(|_| path.to_owned())
        }
        Err(_) => candidate,
    }
}

fn filesystem_access_from_authority(
    authority: &mut adocweave_host::LocalFilesystemPolicy,
    anchor: &Path,
    confined_roots: Vec<PathBuf>,
    independent_roots: Vec<PathBuf>,
    limits: adocweave_host::FilesystemReadLimits,
) -> Result<adocweave_host::LocalFilesystemPolicy, CliError> {
    authority
        .access_derived(
            anchor,
            adocweave_host::DerivedFilesystemRoots {
                confined: confined_roots,
                independent: independent_roots,
            },
            limits,
        )
        .map_err(local_include::LocalIncludeError::Host)
        .map_err(CliError::Include)
}

fn partition_roots_below_anchor(
    anchor: &Path,
    roots: impl IntoIterator<Item = PathBuf>,
    confined: &mut Vec<PathBuf>,
    independent: &mut Vec<PathBuf>,
) {
    for root in roots {
        if root.starts_with(anchor) {
            confined.push(root);
        } else {
            independent.push(root);
        }
    }
}

fn processing_filesystem_roots(
    anchor: &Path,
    primary_roots: impl IntoIterator<Item = PathBuf>,
    arguments: &Arguments,
    allowed_roots: &[PathBuf],
    project_root: Option<&Path>,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut confined = Vec::new();
    let mut independent = Vec::new();
    partition_roots_below_anchor(anchor, primary_roots, &mut confined, &mut independent);
    if arguments.allowed_roots.is_empty() {
        partition_roots_below_anchor(
            anchor,
            allowed_roots.iter().cloned(),
            &mut confined,
            &mut independent,
        );
    } else {
        independent.extend(allowed_roots.iter().cloned());
    }
    if arguments.project_root.is_none() {
        partition_roots_below_anchor(
            anchor,
            project_root.map(Path::to_owned),
            &mut confined,
            &mut independent,
        );
    } else {
        independent.extend(project_root.map(Path::to_owned));
    }
    (confined, independent)
}

fn configuration_stylesheet_session(
    policy: adocweave_host::LocalTargetPolicy,
) -> adocweave_host::LocalTargetSession {
    let stylesheet = adocweave::output::html::StylesheetPolicy::default();
    let max_files = usize::try_from(stylesheet.max_sources).expect("u32 fits usize");
    let max_resource_bytes = u64::from(stylesheet.max_inline_bytes).saturating_add(1);
    adocweave_host::LocalTargetSession::new(
        policy,
        max_files,
        adocweave_host::FilesystemReadLimits {
            max_files,
            max_total_bytes: max_resource_bytes.saturating_mul(u64::from(stylesheet.max_sources)),
            max_resource_bytes,
        },
    )
}

fn validate_resource_plan(
    sizes: impl IntoIterator<Item = u64>,
    plan: adocweave_config::ResolvedResourceLimitPlan,
) -> Result<(), CliError> {
    let mut budget = adocweave_config::AnalysisSnapshotBudget::new(plan.analysis_snapshot);
    for size in sizes {
        budget
            .charge(size)
            .map_err(|error| CliError::ResourceLimit(error.to_string()))?;
    }
    Ok(())
}

fn finish_output(output: String) -> Result<String, CliError> {
    let limit = OutputLimits::default().max_output_bytes;
    if output.len() > usize::try_from(limit).expect("u32 fits usize on supported targets") {
        return Err(CliError::OutputLimit {
            limit,
            actual: u64::try_from(output.len()).expect("usize fits u64"),
        });
    }
    Ok(output)
}

fn load_project_config_at(
    arguments: &Arguments,
    start: &std::path::Path,
    boundary_policy: &adocweave_host::LocalTargetPolicy,
) -> Result<Option<adocweave_config::ConfigSnapshot>, CliError> {
    if arguments.no_config {
        return Ok(None);
    }
    if let Some(path) = &arguments.config_path {
        return adocweave_config::ConfigSnapshot::load_with_preferred_policy(path, boundary_policy)
            .map(Some)
            .map_err(CliError::Config);
    }
    match adocweave_config::discover_and_load_with_policy(start, boundary_policy) {
        Ok(snapshot) => Ok(snapshot),
        Err(error) if error.code == adocweave_config::ConfigErrorCode::OutsideBoundary => Ok(None),
        Err(error) => Err(CliError::Config(error)),
    }
}

fn validate_project_config_authority(
    config: &adocweave_config::ResolvedProjectConfig,
    boundary_policy: &adocweave_host::LocalTargetPolicy,
    resources: bool,
    local_targets: bool,
    stylesheets: bool,
) -> Result<(), CliError> {
    let paths = config
        .resources
        .roots
        .iter()
        .filter(|_| resources)
        .chain(
            config
                .local_targets
                .project_root
                .iter()
                .filter(|_| local_targets),
        )
        .chain(config.html.stylesheet_files.iter().filter(|_| stylesheets));
    for path in paths {
        if boundary_policy.normalize_candidate(path).is_err() {
            return Err(CliError::ConfigAuthority(path.clone()));
        }
    }
    Ok(())
}

/// Decides whether this run resolves `include::` directives.
///
/// Configuration answers first and the command line overrides it in either
/// direction, so a project that turned includes off can still convert one
/// document with them, and a project that leaves the default can convert one
/// document without them.
fn include_selected(arguments: &Arguments, configured: bool) -> bool {
    if arguments.no_include {
        return false;
    }
    arguments.include || configured
}

fn absolute_lexical_path(anchor: &Path, authored: &Path) -> PathBuf {
    let candidate = if authored.is_absolute() {
        authored.to_owned()
    } else {
        anchor.join(authored)
    };
    let mut normalized = PathBuf::new();
    for component in candidate.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn completion_script(shell: CompletionShell) -> String {
    let mut command = command();
    let name = command.get_name().to_owned();
    let mut output = Vec::new();
    let shell = clap_complete::Shell::from(shell);
    clap_complete::generate(shell, &mut command, name, &mut output);
    String::from_utf8(output).expect("clap completion output is UTF-8")
}

fn render_rules_human() -> String {
    let mut rules = diagnostic::LINT_RULES.iter().collect::<Vec<_>>();
    rules.sort_by_key(|rule| rule.id.as_str());
    let mut output = String::from("CODE\tDEFAULT\tSEVERITY\tFIXABLE\tDESCRIPTION\n");
    for rule in rules {
        output.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            rule.id.as_str(),
            if rule.default_enabled {
                "enabled"
            } else {
                "disabled"
            },
            rule.default_severity.as_str(),
            if rule.fixable { "yes" } else { "no" },
            rule.description,
        ));
    }
    output
}

async fn run() -> Result<ExitCode, CliError> {
    match parse_arguments(env::args_os().skip(1))? {
        Action::Lsp => {
            adocweave_lsp::run_stdio()
                .await
                .map_err(CliError::LanguageServer)?;
            Ok(ExitCode::SUCCESS)
        }
        Action::Help(help) => {
            let exit_code = u8::try_from(help.exit_code()).unwrap_or(2);
            help.print().map_err(CliError::Write)?;
            Ok(ExitCode::from(exit_code))
        }
        Action::Version { json } => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "name": "adocweave",
                        "packageVersion": VERSION,
                    })
                );
            } else {
                println!("adocweave {VERSION}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Action::Completion { shell } => {
            print!("{}", completion_script(shell));
            Ok(ExitCode::SUCCESS)
        }
        Action::Rules { json } => {
            if json {
                println!("{}", diagnostic_output::render_lint_rule_catalog_json());
            } else {
                print!("{}", render_rules_human());
            }
            Ok(ExitCode::SUCCESS)
        }
        Action::Run(arguments) => {
            if !matches!(arguments.command, CommandOptions::Preview { .. }) {
                return project_command::run(&arguments);
            }
            let boundary = env::current_dir().map_err(|source| CliError::Read {
                source_name: "current directory".to_owned(),
                source,
            })?;
            let mut filesystem_authority = filesystem_authority(boundary)?;
            let authority_root = filesystem_authority.roots()[0].clone();
            let cli_stdin_base = arguments
                .stdin_base
                .as_ref()
                .map(|path| absolute_lexical_path(&authority_root, path));
            let cli_allowed_roots = arguments
                .allowed_roots
                .iter()
                .map(|path| absolute_lexical_path(&authority_root, path))
                .collect::<Vec<_>>();
            let cli_project_root = arguments
                .project_root
                .as_ref()
                .map(|path| absolute_lexical_path(&authority_root, path));
            let boundary_policy = filesystem_authority
                .root_policy(&authority_root)
                .expect("the initial authority retains its root")
                .clone();
            let input_path = arguments.input.clone();
            let canonical_input = input_path
                .as_ref()
                .map(|path| resolve_primary_path(path, &boundary_policy));
            let config_start = canonical_input.as_deref().unwrap_or(&authority_root);
            let config_snapshot =
                load_project_config_at(&arguments, config_start, &boundary_policy)?;
            let project_config = config_snapshot.as_ref().map_or_else(
                adocweave_config::ResolvedProjectConfig::default,
                |snapshot| snapshot.config.clone(),
            );
            let include = include_selected(&arguments, project_config.resources.include);
            if !include && (arguments.stdin_base.is_some() || !arguments.allowed_roots.is_empty()) {
                return Err(CliError::Usage(
                    "--stdin-base and --allow-root require include processing".to_owned(),
                ));
            }
            let allowed_roots = if arguments.allowed_roots.is_empty() {
                project_config.resources.roots.clone()
            } else {
                cli_allowed_roots
            };
            let project_root = cli_project_root.or_else(|| {
                project_config
                    .local_targets
                    .enabled
                    .then(|| project_config.local_targets.project_root.clone())
                    .flatten()
            });
            validate_project_config_authority(
                &project_config,
                &boundary_policy,
                include && arguments.allowed_roots.is_empty(),
                project_root.is_some() && arguments.project_root.is_none(),
                arguments.command.uses_stylesheets(),
            )?;
            if let CommandOptions::Preview {
                css,
                bind,
                port,
                debounce_ms,
            } = &arguments.command
            {
                let input_path = arguments
                    .input
                    .as_deref()
                    .expect("preview parser requires an input path");
                let (confined_roots, independent_roots) = processing_filesystem_roots(
                    &authority_root,
                    [canonical_input
                        .as_deref()
                        .and_then(Path::parent)
                        .expect("preview input has a parent")
                        .to_owned()],
                    &arguments,
                    &allowed_roots,
                    project_root.as_deref(),
                );
                let preview_filesystem_access = filesystem_access_from_authority(
                    &mut filesystem_authority,
                    &authority_root,
                    confined_roots,
                    independent_roots,
                    project_config.resources.limit_plan.filesystem_reads,
                )?;
                PREVIEW_SHUTDOWN.store(false, std::sync::atomic::Ordering::Release);
                install_preview_signal_handlers();
                commands::preview::run(
                    commands::preview::RunRequest {
                        input_path,
                        include,
                        base_dir: cli_stdin_base.as_deref(),
                        allowed_roots: &allowed_roots,
                        project_root: project_root.as_deref(),
                        project: &project_config,
                        css,
                        configuration_policy: boundary_policy.clone(),
                        filesystem_access: preview_filesystem_access,
                        server: commands::preview::ServerOptions {
                            bind: *bind,
                            port: *port,
                            debounce_ms: *debounce_ms,
                        },
                    },
                    &PREVIEW_SHUTDOWN,
                )
                .map_err(preview_error)?;
                return Ok(ExitCode::SUCCESS);
            }
            unreachable!("preview handled above")
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(exit_code) => exit_code,
        Err(CliError::Arguments(source)) => {
            let exit_code = u8::try_from(source.exit_code()).unwrap_or(2);
            if source.print().is_err() {
                return ExitStatus::InputOutput.into();
            }
            ExitCode::from(exit_code)
        }
        Err(error) => {
            let status = error.exit_status();
            eprintln!("adocweave: {error}");
            // Only a caller who wrote the command wrong is helped by being sent
            // to the help text.
            if status == ExitStatus::Usage {
                eprintln!("Try 'adocweave --help' for more information.");
            }
            status.into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Action, CommandOptions, DEFAULT_PREVIEW_DEBOUNCE_MS, DEFAULT_PREVIEW_PORT, FormatOptions,
        command, parse_arguments,
    };

    fn arguments(values: &[&str]) -> impl Iterator<Item = std::ffi::OsString> {
        values.iter().map(std::ffi::OsString::from)
    }

    #[test]
    fn clap_definition_is_valid_and_every_public_item_has_help() {
        fn assert_documented(command: &clap::Command) {
            assert!(
                command.get_about().is_some() || command.get_long_about().is_some(),
                "{} has no description",
                command.get_name()
            );
            for argument in command.get_arguments() {
                assert!(
                    argument.get_help().is_some() || argument.get_long_help().is_some(),
                    "{}:{} has no help",
                    command.get_name(),
                    argument.get_id()
                );
            }
            for subcommand in command.get_subcommands() {
                assert_documented(subcommand);
            }
        }

        let definition = command();
        definition.clone().debug_assert();
        assert_documented(&definition);
    }

    #[test]
    fn parser_accepts_every_typed_value_candidate() {
        for candidate in ["human", "json", "github", "sarif"] {
            assert!(
                parse_arguments(arguments(&["check", "--format", candidate])).is_ok(),
                "diagnostic format {candidate}"
            );
        }
        for candidate in ["error", "warning", "never"] {
            assert!(
                parse_arguments(arguments(&["check", "--fail-on", candidate])).is_ok(),
                "failure level {candidate}"
            );
        }
        for candidate in ["auto", "always", "never"] {
            assert!(
                parse_arguments(arguments(&["symbols", "--color", candidate])).is_ok(),
                "color choice {candidate}"
            );
        }
    }

    #[test]
    fn parses_file_input() {
        let Action::Run(parsed) =
            parse_arguments(arguments(&["convert", "document.adoc"])).expect("valid arguments")
        else {
            panic!("expected run action");
        };

        assert!(matches!(parsed.command, CommandOptions::Convert { .. }));
        assert_eq!(
            parsed.input.as_deref(),
            Some(std::path::Path::new("document.adoc"))
        );
    }

    #[test]
    fn dash_selects_standard_input() {
        let Action::Run(parsed) =
            parse_arguments(arguments(&["check", "-"])).expect("valid arguments")
        else {
            panic!("expected run action");
        };

        assert!(matches!(parsed.command, CommandOptions::Check(_)));
        assert!(parsed.input.is_none());
    }

    #[test]
    fn all_commands_support_help() {
        for command in [
            "convert",
            "preview",
            "check",
            "format",
            "symbols",
            "rules",
            "completion",
            "lsp",
        ] {
            assert!(matches!(
                parse_arguments(arguments(&[command, "--help"])),
                Ok(Action::Help(_))
            ));
        }
        assert!(matches!(
            parse_arguments(arguments(&["config", "show", "--help"])),
            Ok(Action::Help(_))
        ));
    }

    #[test]
    fn preview_help_explains_options_defaults_and_external_access() {
        let Action::Help(help) =
            parse_arguments(arguments(&["preview", "--help"])).expect("preview help")
        else {
            panic!("expected help action");
        };
        let port = DEFAULT_PREVIEW_PORT.to_string();
        let debounce = DEFAULT_PREVIEW_DEBOUNCE_MS.to_string();
        let help = help.to_string();
        for expected in [
            "--bind <ADDRESS>",
            "127.0.0.1",
            "--port <PORT>",
            "--debounce-ms <MILLISECONDS>",
            "--allow-external",
            "--include",
            "--allow-root <DIR>",
            "--css <FILE>",
            "--css-url <URL>",
            "--config <FILE>",
            "--no-config",
            "--color <WHEN>",
            "auto",
            "authentication",
            "TLS",
        ] {
            assert!(
                help.contains(expected),
                "preview help is missing {expected}"
            );
        }
        for (name, value) in [("port", port), ("debounce", debounce)] {
            assert!(
                help.contains(&value),
                "preview help has the wrong {name} default"
            );
        }

        let Action::Run(parsed) =
            parse_arguments(arguments(&["preview", "document.adoc"])).expect("preview defaults")
        else {
            panic!("expected run action");
        };
        assert!(matches!(
            parsed.command,
            CommandOptions::Preview {
                port: DEFAULT_PREVIEW_PORT,
                debounce_ms: DEFAULT_PREVIEW_DEBOUNCE_MS,
                ..
            }
        ));
    }

    #[test]
    fn preview_requires_a_file_and_explicit_external_authority() {
        assert!(parse_arguments(arguments(&["preview"])).is_err());
        assert!(parse_arguments(arguments(&["preview", "-"])).is_err());
        assert!(
            parse_arguments(arguments(&[
                "preview",
                "--bind",
                "0.0.0.0",
                "document.adoc"
            ]))
            .is_err()
        );
        let Action::Run(parsed) = parse_arguments(arguments(&[
            "preview",
            "--bind",
            "0.0.0.0",
            "--allow-external",
            "--port",
            "8080",
            "--debounce-ms",
            "25",
            "document.adoc",
        ]))
        .expect("explicit external preview") else {
            panic!("expected run action");
        };
        assert!(matches!(
            parsed.command,
            CommandOptions::Preview {
                bind,
                port: 8080,
                debounce_ms: 25,
                ..
            } if bind == "0.0.0.0".parse::<std::net::IpAddr>().expect("address")
        ));
    }

    #[test]
    fn check_rejects_the_removed_json_alias() {
        for values in [
            ["check", "--json", "document.adoc"],
            ["check", "document.adoc", "--json"],
        ] {
            assert!(parse_arguments(arguments(&values)).is_err());
        }
    }

    #[test]
    fn format_accepts_check_flag() {
        let Action::Run(parsed) =
            parse_arguments(arguments(&["format", "--check", "document.adoc"]))
                .expect("valid arguments")
        else {
            panic!("expected run action");
        };
        assert!(matches!(
            parsed.command,
            CommandOptions::Format(FormatOptions { check: true, .. })
        ));
    }

    #[test]
    fn include_options_are_explicit_and_repeatable() {
        let Action::Run(parsed) = parse_arguments(arguments(&[
            "convert",
            "--include",
            "--stdin-base",
            "docs",
            "--allow-root",
            ".",
            "--allow-root",
            "vendor",
            "-",
        ]))
        .expect("valid arguments") else {
            panic!("expected run action");
        };
        assert!(parsed.include);
        assert_eq!(
            parsed.stdin_base.as_deref(),
            Some(std::path::Path::new("docs"))
        );
        assert_eq!(parsed.allowed_roots.len(), 2);
    }
}
