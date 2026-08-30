use std::env;
use std::sync::atomic::AtomicBool;

use adocweave::OutputLimits;
use adocweave::output::diagnostics as diagnostic;

mod arguments;
mod check_output;
mod cli_error;
mod commands;
mod diagnostic_json;
mod diagnostic_output;
mod exit_code;
mod file_workflow;
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

use arguments::{Action, CommandOptions, CompletionShell, command, parse_arguments};
use cli_error::{CliError, preview_error};
use exit_code::CliExitCode;

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

async fn run() -> Result<CliExitCode, CliError> {
    match parse_arguments(env::args_os().skip(1))? {
        Action::Lsp => {
            adocweave_lsp::run_stdio()
                .await
                .map_err(CliError::LanguageServer)?;
            Ok(CliExitCode::Success)
        }
        Action::Help(help) => {
            let exit_code = CliExitCode::from_clap(help.exit_code());
            help.print().map_err(CliError::Write)?;
            Ok(exit_code)
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
            Ok(CliExitCode::Success)
        }
        Action::Completion { shell } => {
            print!("{}", completion_script(shell));
            Ok(CliExitCode::Success)
        }
        Action::Rules { json } => {
            if json {
                println!("{}", diagnostic_output::render_lint_rule_catalog_json());
            } else {
                print!("{}", render_rules_human());
            }
            Ok(CliExitCode::Success)
        }
        Action::Run(arguments) => {
            if !matches!(arguments.command, CommandOptions::Preview { .. }) {
                return project_command::run(&arguments);
            }
            if let CommandOptions::Preview { css, .. } = &arguments.command {
                commands::html_policy::validate_argument_count(css)
                    .map_err(cli_error::html_policy_error)?;
            }
            let current = env::current_dir().map_err(|source| CliError::Read {
                source_name: "current directory".to_owned(),
                source,
            })?;
            let current = current.canonicalize().map_err(|source| CliError::Read {
                source_name: "current directory".to_owned(),
                source,
            })?;
            let authority = project_command::project_authority(&arguments, &current)?;
            let watch = commands::preview::PreviewWatchAccess::from_authority(&authority);
            let project = project_command::request_with_authority(&arguments, &current, authority)?;
            if let CommandOptions::Preview {
                css,
                bind,
                port,
                debounce_ms,
            } = &arguments.command
            {
                PREVIEW_SHUTDOWN.store(false, std::sync::atomic::Ordering::Release);
                install_preview_signal_handlers();
                commands::preview::run(
                    commands::preview::RunRequest {
                        project,
                        watch,
                        css,
                        server: commands::preview::ServerOptions {
                            bind: *bind,
                            port: *port,
                            debounce_ms: *debounce_ms,
                        },
                    },
                    &PREVIEW_SHUTDOWN,
                )
                .map_err(preview_error)?;
                return Ok(CliExitCode::Success);
            }
            unreachable!("preview handled above")
        }
    }
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(exit_code) => exit_code.into(),
        Err(CliError::Arguments(source)) => {
            let exit_code = CliExitCode::from_clap(source.exit_code());
            if source.print().is_err() {
                return CliExitCode::InputOutput.into();
            }
            exit_code.into()
        }
        Err(error) => {
            let exit_code = error.exit_code();
            eprintln!("adocweave: {error}");
            // Only a caller who wrote the command wrong is helped by being sent
            // to the help text.
            if exit_code == CliExitCode::Usage {
                eprintln!("Try 'adocweave --help' for more information.");
            }
            exit_code.into()
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
