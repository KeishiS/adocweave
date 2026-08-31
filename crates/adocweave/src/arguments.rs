//! Command-line definition and conversion to the application's input types.

use std::ffi::{OsStr, OsString};
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use adocweave_core::output::diagnostics as diagnostic;
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum, ValueHint};

use crate::check_output::{DiagnosticFormat, FailOn};
use crate::cli_error::CliError;
use crate::commands::check::Options as CheckOptions;
use crate::commands::format::Options as FormatOptions;
use crate::commands::html_policy::StylesheetArgument;
use crate::{DEFAULT_PREVIEW_DEBOUNCE_MS, DEFAULT_PREVIEW_PORT};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub(crate) enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Debug)]
pub(crate) enum CommandOptions {
    Convert {
        complete: bool,
        css: Vec<StylesheetArgument>,
    },
    Preview {
        css: Vec<StylesheetArgument>,
        bind: IpAddr,
        port: u16,
        debounce_ms: u64,
    },
    Check(CheckOptions),
    Format(FormatOptions),
    Symbols,
    ConfigShow,
}

pub(crate) struct Arguments {
    pub(crate) command: CommandOptions,
    pub(crate) input: Option<PathBuf>,
    pub(crate) additional_inputs: Vec<PathBuf>,
    pub(crate) glob_patterns: Vec<String>,
    pub(crate) include: bool,
    pub(crate) no_include: bool,
    pub(crate) stdin_base: Option<PathBuf>,
    pub(crate) allowed_roots: Vec<PathBuf>,
    pub(crate) project_root: Option<PathBuf>,
    pub(crate) config_path: Option<PathBuf>,
    pub(crate) no_config: bool,
    pub(crate) color: ColorChoice,
}

pub(crate) enum Action {
    Run(Box<Arguments>),
    Lsp,
    Help(clap::Error),
    Version { json: bool },
    Completion { shell: CompletionShell },
    Rules { json: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    #[value(name = "powershell")]
    PowerShell,
}

impl From<CompletionShell> for clap_complete::Shell {
    fn from(value: CompletionShell) -> Self {
        match value {
            CompletionShell::Bash => Self::Bash,
            CompletionShell::Zsh => Self::Zsh,
            CompletionShell::Fish => Self::Fish,
            CompletionShell::PowerShell => Self::PowerShell,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "adocweave",
    version,
    about = "AdocWeave command-line interface",
    disable_version_flag = true,
    arg_required_else_help = true,
    args_conflicts_with_subcommands = true
)]
struct Cli {
    /// Print version information.
    #[arg(short = 'V', long)]
    version: bool,

    /// Use JSON output with --version.
    #[arg(long, requires = "version")]
    json: bool,

    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Convert an AsciiDoc document to HTML.
    #[command(after_help = "Example:\n  adocweave convert --complete manual.adoc")]
    Convert(ConvertArgs),

    /// Serve a live document preview.
    #[command(
        after_help = "Security:\n  A non-loopback address requires --allow-external.\n  The server does not provide authentication or TLS encryption.\n\nExample:\n  adocweave preview --port 8080 manual.adoc"
    )]
    Preview(PreviewArgs),

    /// Check AsciiDoc documents.
    #[command(
        after_help = "Examples:\n  adocweave check --fail-on warning docs\n  adocweave check --format sarif docs > adocweave.sarif\n  adocweave check --fix docs\n  adocweave check --fix --diff docs"
    )]
    Check(CheckArgs),

    /// Format AsciiDoc documents.
    #[command(
        after_help = "Examples:\n  adocweave format --check docs\n  adocweave format --diff manual.adoc\n  adocweave format --write docs"
    )]
    Format(FormatArgs),

    /// Print document symbols as JSON.
    #[command(after_help = "Example:\n  adocweave symbols manual.adoc")]
    Symbols(SymbolArgs),

    /// List diagnostic rules.
    #[command(after_help = "Examples:\n  adocweave rules\n  adocweave rules --format json")]
    Rules(RuleArgs),

    /// Inspect project configuration.
    Config(ConfigArgs),

    /// Print a shell completion script.
    #[command(after_help = "Example:\n  adocweave completion bash")]
    Completion(CompletionArgs),

    /// Run the Language Server over standard input and output.
    Lsp,
}

#[derive(Debug, Args)]
struct ProjectConfigArgs {
    /// Use the specified project configuration.
    #[arg(long, value_name = "FILE", value_hint = ValueHint::FilePath, conflicts_with = "no_config")]
    config: Option<PathBuf>,

    /// Disable project configuration discovery.
    #[arg(long)]
    no_config: bool,
}

#[derive(Debug, Args)]
struct IncludeArgs {
    /// Process local includes even if disabled by configuration.
    #[arg(long, conflicts_with = "no_include")]
    include: bool,

    /// Leave include directives unresolved.
    #[arg(long)]
    no_include: bool,
}

#[derive(Debug, Args)]
struct AllowedRootArgs {
    /// Permit includes below this directory; repeatable.
    #[arg(long = "allow-root", value_name = "DIR", num_args = 1, value_hint = ValueHint::DirPath)]
    allowed_roots: Vec<PathBuf>,
}

#[derive(Debug, Args)]
struct StdinArgs {
    /// Resolve standard-input includes from this directory.
    #[arg(long = "stdin-base", value_name = "DIR", value_hint = ValueHint::DirPath)]
    stdin_base: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ColorArgs {
    /// Control color in human-readable diagnostics and diffs.
    #[arg(long, value_name = "WHEN", value_enum, default_value_t)]
    color: ColorChoice,
}

#[derive(Debug, Args)]
struct ConvertArgs {
    /// Input file; omit or use - for standard input.
    #[arg(value_name = "FILE", value_hint = ValueHint::FilePath)]
    file: Option<PathBuf>,

    /// Output a complete HTML document.
    #[arg(long)]
    complete: bool,

    /// Embed CSS from this file; repeatable.
    #[arg(long, value_name = "FILE", num_args = 1, value_hint = ValueHint::FilePath)]
    css: Vec<PathBuf>,

    /// Link an allowed CSS URL; repeatable.
    #[arg(long = "css-url", value_name = "URL", num_args = 1, value_hint = ValueHint::Url)]
    css_url: Vec<String>,

    #[command(flatten)]
    include: IncludeArgs,
    #[command(flatten)]
    stdin: StdinArgs,
    #[command(flatten)]
    roots: AllowedRootArgs,
    #[command(flatten)]
    config: ProjectConfigArgs,
    #[command(flatten)]
    color: ColorArgs,
}

#[derive(Debug, Args)]
struct PreviewArgs {
    /// AsciiDoc file to preview; standard input and symbolic links are not supported.
    #[arg(value_name = "FILE", value_hint = ValueHint::FilePath)]
    file: PathBuf,

    /// Listen address.
    #[arg(long, value_name = "ADDRESS", default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
    bind: IpAddr,

    /// Listen port.
    #[arg(long, value_name = "PORT", default_value_t = DEFAULT_PREVIEW_PORT)]
    port: u16,

    /// Rebuild debounce interval in milliseconds.
    #[arg(long = "debounce-ms", value_name = "MILLISECONDS", default_value_t = DEFAULT_PREVIEW_DEBOUNCE_MS, value_parser = positive_u64)]
    debounce_ms: u64,

    /// Permit an explicitly selected non-loopback address.
    #[arg(long)]
    allow_external: bool,

    /// Embed CSS from this file; repeatable.
    #[arg(long, value_name = "FILE", num_args = 1, value_hint = ValueHint::FilePath)]
    css: Vec<PathBuf>,

    /// Link an allowed CSS URL; repeatable.
    #[arg(long = "css-url", value_name = "URL", num_args = 1, value_hint = ValueHint::Url)]
    css_url: Vec<String>,

    #[command(flatten)]
    include: IncludeArgs,
    #[command(flatten)]
    roots: AllowedRootArgs,
    #[command(flatten)]
    config: ProjectConfigArgs,
    #[command(flatten)]
    color: ColorArgs,
}

#[derive(Debug, Args)]
struct CheckArgs {
    /// Input file or directory; omit or use - for standard input.
    #[arg(value_name = "FILE", value_hint = ValueHint::AnyPath)]
    files: Vec<PathBuf>,

    /// Select the diagnostic output format.
    #[arg(long, value_name = "FORMAT", value_enum, default_value_t)]
    format: DiagnosticFormat,

    /// Set when diagnostics cause a nonzero exit status.
    #[arg(long = "fail-on", value_name = "LEVEL", value_enum, default_value_t)]
    fail_on: FailOn,

    /// Print counts to standard error.
    #[arg(long)]
    summary: bool,

    /// Write fixes that are always safe to apply.
    #[arg(long)]
    fix: bool,

    /// Print proposed fixes as a unified diff without modifying files.
    #[arg(long, requires = "fix")]
    diff: bool,

    /// Enable an opt-in rule; repeatable.
    #[arg(long = "enable-rule", value_name = "CODE", num_args = 1)]
    enabled_rules: Vec<String>,

    /// Also process files matching this pattern; repeatable.
    #[arg(long = "glob", value_name = "PATTERN", num_args = 1)]
    glob_patterns: Vec<String>,

    /// Check local file targets below this directory.
    #[arg(
        long = "project-root",
        value_name = "DIR",
        value_hint = ValueHint::DirPath,
        conflicts_with = "allowed_roots"
    )]
    project_root: Option<PathBuf>,

    #[command(flatten)]
    include: IncludeArgs,
    #[command(flatten)]
    stdin: StdinArgs,
    #[command(flatten)]
    roots: AllowedRootArgs,
    #[command(flatten)]
    config: ProjectConfigArgs,
    #[command(flatten)]
    color: ColorArgs,
}

#[derive(Debug, Args)]
struct FormatArgs {
    /// Input file or directory; omit or use - for standard input.
    #[arg(value_name = "FILE", value_hint = ValueHint::AnyPath)]
    files: Vec<PathBuf>,

    /// Check whether formatting changes are required.
    #[arg(long, conflicts_with_all = ["write", "diff"])]
    check: bool,

    /// Write formatted output to files.
    #[arg(long, conflicts_with_all = ["check", "diff"])]
    write: bool,

    /// Print changes as a unified diff.
    #[arg(long, conflicts_with_all = ["check", "write"])]
    diff: bool,

    /// Print counts to standard error.
    #[arg(long)]
    summary: bool,

    /// Also process files matching this pattern; repeatable.
    #[arg(long = "glob", value_name = "PATTERN", num_args = 1)]
    glob_patterns: Vec<String>,

    #[command(flatten)]
    include: IncludeArgs,
    #[command(flatten)]
    stdin: StdinArgs,
    #[command(flatten)]
    roots: AllowedRootArgs,
    #[command(flatten)]
    config: ProjectConfigArgs,
    #[command(flatten)]
    color: ColorArgs,
}

#[derive(Debug, Args)]
struct SymbolArgs {
    /// Input file; omit or use - for standard input.
    #[arg(value_name = "FILE", value_hint = ValueHint::FilePath)]
    file: Option<PathBuf>,

    #[command(flatten)]
    include: IncludeArgs,
    #[command(flatten)]
    stdin: StdinArgs,
    #[command(flatten)]
    roots: AllowedRootArgs,
    #[command(flatten)]
    config: ProjectConfigArgs,
    #[command(flatten)]
    color: ColorArgs,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum RuleOutput {
    #[default]
    Human,
    Json,
}

#[derive(Debug, Args)]
struct RuleArgs {
    /// Select the output format.
    #[arg(long, value_name = "FORMAT", value_enum, default_value_t)]
    format: RuleOutput,
}

#[derive(Debug, Args)]
struct ConfigArgs {
    #[command(subcommand)]
    command: ConfigCommand,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Print the resolved project configuration as JSON.
    #[command(after_help = "Example:\n  adocweave config show")]
    Show(ProjectConfigArgs),
}

#[derive(Debug, Args)]
struct CompletionArgs {
    /// Shell for which to generate completion.
    #[arg(value_enum)]
    shell: CompletionShell,
}

fn positive_u64(value: &str) -> Result<u64, String> {
    let value = value
        .parse::<u64>()
        .map_err(|_| "expected a positive integer".to_owned())?;
    if value == 0 {
        Err("value must be greater than zero".to_owned())
    } else {
        Ok(value)
    }
}

fn stylesheet_arguments(matches: &clap::ArgMatches) -> Vec<StylesheetArgument> {
    let mut values = Vec::new();
    if let (Some(indices), Some(stylesheets)) = (
        matches.indices_of("css"),
        matches.get_many::<PathBuf>("css"),
    ) {
        values.extend(
            indices
                .zip(stylesheets)
                .map(|(index, path)| (index, StylesheetArgument::File(path.clone()))),
        );
    }
    if let (Some(indices), Some(stylesheets)) = (
        matches.indices_of("css_url"),
        matches.get_many::<String>("css_url"),
    ) {
        values.extend(
            indices
                .zip(stylesheets)
                .map(|(index, url)| (index, StylesheetArgument::Url(url.clone()))),
        );
    }
    values.sort_by_key(|(index, _)| *index);
    values.into_iter().map(|(_, value)| value).collect()
}

fn single_input(value: Option<PathBuf>) -> Option<PathBuf> {
    value.filter(|value| value.as_os_str() != OsStr::new("-"))
}

fn multiple_inputs(values: Vec<PathBuf>) -> Result<(Option<PathBuf>, Vec<PathBuf>), CliError> {
    if values.is_empty() || (values.len() == 1 && values[0].as_os_str() == OsStr::new("-")) {
        return Ok((None, Vec::new()));
    }
    if values
        .iter()
        .any(|value| value.as_os_str() == OsStr::new("-"))
    {
        return Err(CliError::Usage(
            "standard input cannot be combined with file paths".to_owned(),
        ));
    }
    let mut values = values.into_iter();
    Ok((values.next(), values.collect()))
}

fn enabled_rules(values: Vec<String>) -> Result<Vec<diagnostic::LintRuleId>, CliError> {
    let mut rules = Vec::new();
    for code in values {
        let descriptor = diagnostic::lint_rule(&code)
            .ok_or_else(|| CliError::Usage(format!("unknown or non-enableable rule: {code}")))?;
        if descriptor.default_enabled {
            return Err(CliError::Usage(format!(
                "rule is already enabled by default: {code}"
            )));
        }
        if !rules.contains(&descriptor.id) {
            rules.push(descriptor.id);
        }
    }
    Ok(rules)
}

fn run_action(arguments: Arguments) -> Result<Action, CliError> {
    if arguments.stdin_base.is_some() && arguments.input.is_some() {
        return Err(CliError::Usage(
            "--stdin-base can be used only with standard input".to_owned(),
        ));
    }
    if arguments.input.is_none() && !arguments.glob_patterns.is_empty() {
        return Err(CliError::Usage(
            "standard input cannot be combined with --glob".to_owned(),
        ));
    }
    Ok(Action::Run(Box::new(arguments)))
}

fn convert_action(command: ConvertArgs, matches: &clap::ArgMatches) -> Result<Action, CliError> {
    run_action(Arguments {
        command: CommandOptions::Convert {
            complete: command.complete,
            css: stylesheet_arguments(matches),
        },
        input: single_input(command.file),
        additional_inputs: Vec::new(),
        glob_patterns: Vec::new(),
        include: command.include.include,
        no_include: command.include.no_include,
        stdin_base: command.stdin.stdin_base,
        allowed_roots: command.roots.allowed_roots,
        project_root: None,
        config_path: command.config.config,
        no_config: command.config.no_config,
        color: command.color.color,
    })
}

fn preview_action(command: PreviewArgs, matches: &clap::ArgMatches) -> Result<Action, CliError> {
    if command.file.as_os_str() == OsStr::new("-") {
        return Err(CliError::Usage(
            "standard input is not supported by preview".to_owned(),
        ));
    }
    if !command.bind.is_loopback() && !command.allow_external {
        return Err(CliError::Usage(
            "a non-loopback --bind requires --allow-external".to_owned(),
        ));
    }
    run_action(Arguments {
        command: CommandOptions::Preview {
            css: stylesheet_arguments(matches),
            bind: command.bind,
            port: command.port,
            debounce_ms: command.debounce_ms,
        },
        input: Some(command.file),
        additional_inputs: Vec::new(),
        glob_patterns: Vec::new(),
        include: command.include.include,
        no_include: command.include.no_include,
        stdin_base: None,
        allowed_roots: command.roots.allowed_roots,
        project_root: None,
        config_path: command.config.config,
        no_config: command.config.no_config,
        color: command.color.color,
    })
}

fn check_action(command: CheckArgs) -> Result<Action, CliError> {
    let (input, additional_inputs) = multiple_inputs(command.files)?;
    if command.fix && input.is_none() {
        return Err(CliError::Usage(
            "check --fix requires at least one file or directory".to_owned(),
        ));
    }
    let format = command.format;
    if command.diff && format != DiagnosticFormat::Human {
        return Err(CliError::Usage(
            "check --fix --diff requires --format human".to_owned(),
        ));
    }
    run_action(Arguments {
        command: CommandOptions::Check(CheckOptions {
            format,
            fail_on: command.fail_on,
            summary: command.summary,
            fix: command.fix,
            diff: command.diff,
            enabled_rules: enabled_rules(command.enabled_rules)?,
        }),
        input,
        additional_inputs,
        glob_patterns: command.glob_patterns,
        include: command.include.include,
        no_include: command.include.no_include,
        stdin_base: command.stdin.stdin_base,
        allowed_roots: command.roots.allowed_roots,
        project_root: command.project_root,
        config_path: command.config.config,
        no_config: command.config.no_config,
        color: command.color.color,
    })
}

fn format_action(command: FormatArgs) -> Result<Action, CliError> {
    let (input, additional_inputs) = multiple_inputs(command.files)?;
    run_action(Arguments {
        command: CommandOptions::Format(FormatOptions {
            check: command.check,
            write: command.write,
            diff: command.diff,
            summary: command.summary,
        }),
        input,
        additional_inputs,
        glob_patterns: command.glob_patterns,
        include: command.include.include,
        no_include: command.include.no_include,
        stdin_base: command.stdin.stdin_base,
        allowed_roots: command.roots.allowed_roots,
        project_root: None,
        config_path: command.config.config,
        no_config: command.config.no_config,
        color: command.color.color,
    })
}

fn symbols_action(command: SymbolArgs) -> Result<Action, CliError> {
    run_action(Arguments {
        command: CommandOptions::Symbols,
        input: single_input(command.file),
        additional_inputs: Vec::new(),
        glob_patterns: Vec::new(),
        include: command.include.include,
        no_include: command.include.no_include,
        stdin_base: command.stdin.stdin_base,
        allowed_roots: command.roots.allowed_roots,
        project_root: None,
        config_path: command.config.config,
        no_config: command.config.no_config,
        color: command.color.color,
    })
}

pub(crate) fn command() -> clap::Command {
    Cli::command()
}

pub(crate) fn parse_arguments<T>(arguments: impl Iterator<Item = T>) -> Result<Action, CliError>
where
    T: Into<OsString>,
{
    let mut definition = Cli::command();
    let matches = match definition.try_get_matches_from_mut(
        std::iter::once(OsString::from("adocweave")).chain(arguments.map(Into::into)),
    ) {
        Ok(matches) => matches,
        Err(error) if !error.use_stderr() => {
            return Ok(Action::Help(error));
        }
        Err(error) => return Err(CliError::Arguments(error)),
    };
    let cli = Cli::from_arg_matches(&matches).map_err(CliError::Arguments)?;
    if cli.version {
        return Ok(Action::Version { json: cli.json });
    }
    let command = cli
        .command
        .expect("clap requires a command unless --version exits first");
    let (_, command_matches) = matches
        .subcommand()
        .expect("a parsed command has matching arguments");
    match command {
        CliCommand::Convert(command) => convert_action(command, command_matches),
        CliCommand::Preview(command) => preview_action(command, command_matches),
        CliCommand::Check(command) => check_action(command),
        CliCommand::Format(command) => format_action(command),
        CliCommand::Symbols(command) => symbols_action(command),
        CliCommand::Rules(command) => Ok(Action::Rules {
            json: command.format == RuleOutput::Json,
        }),
        CliCommand::Config(ConfigArgs {
            command: ConfigCommand::Show(config),
        }) => Ok(Action::Run(Box::new(Arguments {
            command: CommandOptions::ConfigShow,
            input: None,
            additional_inputs: Vec::new(),
            glob_patterns: Vec::new(),
            include: false,
            no_include: false,
            stdin_base: None,
            allowed_roots: Vec::new(),
            project_root: None,
            config_path: config.config,
            no_config: config.no_config,
            color: ColorChoice::Auto,
        }))),
        CliCommand::Completion(command) => Ok(Action::Completion {
            shell: command.shell,
        }),
        CliCommand::Lsp => Ok(Action::Lsp),
    }
}
