#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum CommandId {
    Convert,
    Preview,
    Check,
    Format,
    Symbols,
    Rules,
    ConfigShow,
    Completion,
    Help,
}

const DOCUMENT_COMMANDS: &[CommandId] = &[
    CommandId::Convert,
    CommandId::Preview,
    CommandId::Check,
    CommandId::Format,
    CommandId::Symbols,
    CommandId::ConfigShow,
];
const INPUT_COMMANDS: &[CommandId] = &[
    CommandId::Convert,
    CommandId::Preview,
    CommandId::Check,
    CommandId::Format,
    CommandId::Symbols,
];
const STDIN_COMMANDS: &[CommandId] = &[
    CommandId::Convert,
    CommandId::Check,
    CommandId::Format,
    CommandId::Symbols,
];
const HELP_COMMANDS: &[CommandId] = &[
    CommandId::Convert,
    CommandId::Preview,
    CommandId::Check,
    CommandId::Format,
    CommandId::Symbols,
    CommandId::Rules,
    CommandId::ConfigShow,
    CommandId::Completion,
];
const CHECK_AND_FORMAT: &[CommandId] = &[CommandId::Check, CommandId::Format];
const CONVERT_AND_PREVIEW: &[CommandId] = &[CommandId::Convert, CommandId::Preview];

const DIAGNOSTIC_FORMATS: &[&str] = &["human", "json", "github", "sarif"];
const RULE_FORMATS: &[&str] = &["human", "json"];
const FAILURE_LEVELS: &[&str] = &["error", "warning", "never"];
const COLOR_CHOICES: &[&str] = &["auto", "always", "never"];

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum OptionId {
    DiagnosticFormat,
    RuleFormat,
    Json,
    FailOn,
    Summary,
    Fix,
    Config,
    NoConfig,
    EnableRule,
    FormatCheck,
    FormatWrite,
    Diff,
    Glob,
    Color,
    Include,
    NoInclude,
    StdinBase,
    AllowRoot,
    ProjectRoot,
    Complete,
    Css,
    CssUrl,
    Bind,
    Port,
    Debounce,
    AllowExternal,
    Version,
    Help,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OptionValue {
    Flag,
    Required {
        metavar: &'static str,
        missing: &'static str,
        candidates: &'static [&'static str],
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OptionSpec {
    pub(crate) id: OptionId,
    pub(crate) names: &'static [&'static str],
    pub(crate) value: OptionValue,
    pub(crate) commands: &'static [CommandId],
    pub(crate) root: bool,
    pub(crate) version: bool,
    pub(crate) help_line: &'static str,
}

impl OptionSpec {
    pub(crate) fn canonical_name(&self) -> &'static str {
        self.names
            .iter()
            .copied()
            .find(|name| name.starts_with("--"))
            .expect("every option has a long name")
    }

    pub(crate) fn applies_to(&self, command: CommandId) -> bool {
        self.commands.contains(&command)
    }

    pub(crate) fn metavar(&self) -> Option<&'static str> {
        match self.value {
            OptionValue::Flag => None,
            OptionValue::Required { metavar, .. } => Some(metavar),
        }
    }

    pub(crate) fn missing_value(&self) -> Option<&'static str> {
        match self.value {
            OptionValue::Flag => None,
            OptionValue::Required { missing, .. } => Some(missing),
        }
    }

    pub(crate) fn candidates(&self) -> &'static [&'static str] {
        match self.value {
            OptionValue::Flag => &[],
            OptionValue::Required { candidates, .. } => candidates,
        }
    }
}

pub(crate) const OPTIONS: &[OptionSpec] = &[
    OptionSpec {
        id: OptionId::DiagnosticFormat,
        names: &["--format"],
        value: OptionValue::Required {
            metavar: "FORMAT",
            missing: "a value",
            candidates: DIAGNOSTIC_FORMATS,
        },
        commands: &[CommandId::Check],
        root: false,
        version: false,
        help_line: "  --format FORMAT  Output format: human, json, github, or sarif (default: human)\n",
    },
    OptionSpec {
        id: OptionId::RuleFormat,
        names: &["--format"],
        value: OptionValue::Required {
            metavar: "FORMAT",
            missing: "a value",
            candidates: RULE_FORMATS,
        },
        commands: &[CommandId::Rules],
        root: false,
        version: false,
        help_line: "  --format FORMAT  Output format: human or json (default: human)\n",
    },
    OptionSpec {
        id: OptionId::Json,
        names: &["--json"],
        value: OptionValue::Flag,
        commands: &[],
        root: false,
        version: true,
        help_line: "  --json  Print version information as JSON\n",
    },
    OptionSpec {
        id: OptionId::FailOn,
        names: &["--fail-on"],
        value: OptionValue::Required {
            metavar: "LEVEL",
            missing: "a level",
            candidates: FAILURE_LEVELS,
        },
        commands: &[CommandId::Check],
        root: false,
        version: false,
        help_line: "  --fail-on LEVEL  Fail on error, warning, or never (default: error)\n",
    },
    OptionSpec {
        id: OptionId::Summary,
        names: &["--summary"],
        value: OptionValue::Flag,
        commands: CHECK_AND_FORMAT,
        root: false,
        version: false,
        help_line: "  --summary  Print counts to standard error\n",
    },
    OptionSpec {
        id: OptionId::Fix,
        names: &["--fix"],
        value: OptionValue::Flag,
        commands: &[CommandId::Check],
        root: false,
        version: false,
        help_line: "  --fix  Write fixes that are always safe to apply\n",
    },
    OptionSpec {
        id: OptionId::Config,
        names: &["--config"],
        value: OptionValue::Required {
            metavar: "FILE",
            missing: "a file",
            candidates: &[],
        },
        commands: DOCUMENT_COMMANDS,
        root: false,
        version: false,
        help_line: "  --config FILE  Use the specified project configuration\n",
    },
    OptionSpec {
        id: OptionId::NoConfig,
        names: &["--no-config"],
        value: OptionValue::Flag,
        commands: DOCUMENT_COMMANDS,
        root: false,
        version: false,
        help_line: "  --no-config  Disable project configuration discovery\n",
    },
    OptionSpec {
        id: OptionId::EnableRule,
        names: &["--enable-rule"],
        value: OptionValue::Required {
            metavar: "CODE",
            missing: "a code",
            candidates: &[],
        },
        commands: &[CommandId::Check],
        root: false,
        version: false,
        help_line: "  --enable-rule CODE  Enable an opt-in rule; repeatable\n",
    },
    OptionSpec {
        id: OptionId::FormatCheck,
        names: &["--check"],
        value: OptionValue::Flag,
        commands: &[CommandId::Format],
        root: false,
        version: false,
        help_line: "  --check  Check whether formatting changes are required\n",
    },
    OptionSpec {
        id: OptionId::FormatWrite,
        names: &["--write"],
        value: OptionValue::Flag,
        commands: &[CommandId::Format],
        root: false,
        version: false,
        help_line: "  --write  Write formatted output to files\n",
    },
    OptionSpec {
        id: OptionId::Diff,
        names: &["--diff"],
        value: OptionValue::Flag,
        commands: CHECK_AND_FORMAT,
        root: false,
        version: false,
        help_line: "  --diff  Print changes as a unified diff\n",
    },
    OptionSpec {
        id: OptionId::Glob,
        names: &["--glob"],
        value: OptionValue::Required {
            metavar: "PATTERN",
            missing: "a pattern",
            candidates: &[],
        },
        commands: CHECK_AND_FORMAT,
        root: false,
        version: false,
        help_line: "  --glob PATTERN  Add matching files; repeatable\n",
    },
    OptionSpec {
        id: OptionId::Color,
        names: &["--color"],
        value: OptionValue::Required {
            metavar: "WHEN",
            missing: "a value",
            candidates: COLOR_CHOICES,
        },
        commands: INPUT_COMMANDS,
        root: false,
        version: false,
        help_line: "  --color WHEN  Color output: auto, always, or never (default: auto)\n",
    },
    OptionSpec {
        id: OptionId::Include,
        names: &["--include"],
        value: OptionValue::Flag,
        commands: INPUT_COMMANDS,
        root: false,
        version: false,
        help_line: "  --include  Process local includes even if disabled by configuration\n",
    },
    OptionSpec {
        id: OptionId::NoInclude,
        names: &["--no-include"],
        value: OptionValue::Flag,
        commands: INPUT_COMMANDS,
        root: false,
        version: false,
        help_line: "  --no-include  Leave include directives unresolved\n",
    },
    OptionSpec {
        id: OptionId::StdinBase,
        names: &["--stdin-base"],
        value: OptionValue::Required {
            metavar: "DIR",
            missing: "a directory",
            candidates: &[],
        },
        commands: STDIN_COMMANDS,
        root: false,
        version: false,
        help_line: "  --stdin-base DIR  Resolve standard-input includes from DIR\n",
    },
    OptionSpec {
        id: OptionId::AllowRoot,
        names: &["--allow-root"],
        value: OptionValue::Required {
            metavar: "DIR",
            missing: "a directory",
            candidates: &[],
        },
        commands: INPUT_COMMANDS,
        root: false,
        version: false,
        help_line: "  --allow-root DIR  Permit includes below DIR; repeatable\n",
    },
    OptionSpec {
        id: OptionId::ProjectRoot,
        names: &["--project-root"],
        value: OptionValue::Required {
            metavar: "DIR",
            missing: "a directory",
            candidates: &[],
        },
        commands: &[CommandId::Check],
        root: false,
        version: false,
        help_line: "  --project-root DIR  Check local file targets below DIR\n",
    },
    OptionSpec {
        id: OptionId::Complete,
        names: &["--complete"],
        value: OptionValue::Flag,
        commands: &[CommandId::Convert],
        root: false,
        version: false,
        help_line: "  --complete  Output a complete HTML document\n",
    },
    OptionSpec {
        id: OptionId::Css,
        names: &["--css"],
        value: OptionValue::Required {
            metavar: "FILE",
            missing: "a file",
            candidates: &[],
        },
        commands: CONVERT_AND_PREVIEW,
        root: false,
        version: false,
        help_line: "  --css FILE  Embed CSS from FILE; repeatable\n",
    },
    OptionSpec {
        id: OptionId::CssUrl,
        names: &["--css-url"],
        value: OptionValue::Required {
            metavar: "URL",
            missing: "a URL",
            candidates: &[],
        },
        commands: CONVERT_AND_PREVIEW,
        root: false,
        version: false,
        help_line: "  --css-url URL  Link an allowed CSS URL; repeatable\n",
    },
    OptionSpec {
        id: OptionId::Bind,
        names: &["--bind"],
        value: OptionValue::Required {
            metavar: "ADDRESS",
            missing: "an address",
            candidates: &[],
        },
        commands: &[CommandId::Preview],
        root: false,
        version: false,
        help_line: "  --bind ADDRESS  Listen address (default: 127.0.0.1)\n",
    },
    OptionSpec {
        id: OptionId::Port,
        names: &["--port"],
        value: OptionValue::Required {
            metavar: "PORT",
            missing: "a value",
            candidates: &[],
        },
        commands: &[CommandId::Preview],
        root: false,
        version: false,
        help_line: "  --port PORT  Listen port (default: 4000)\n",
    },
    OptionSpec {
        id: OptionId::Debounce,
        names: &["--debounce-ms"],
        value: OptionValue::Required {
            metavar: "MILLISECONDS",
            missing: "a value",
            candidates: &[],
        },
        commands: &[CommandId::Preview],
        root: false,
        version: false,
        help_line: "  --debounce-ms MILLISECONDS  Rebuild debounce interval (default: 100)\n",
    },
    OptionSpec {
        id: OptionId::AllowExternal,
        names: &["--allow-external"],
        value: OptionValue::Flag,
        commands: &[CommandId::Preview],
        root: false,
        version: false,
        help_line: "  --allow-external  Permit an explicitly selected non-loopback address\n",
    },
    OptionSpec {
        id: OptionId::Version,
        names: &["-V", "--version"],
        value: OptionValue::Flag,
        commands: &[],
        root: true,
        version: false,
        help_line: "  -V, --version [--json]  Print version\n",
    },
    OptionSpec {
        id: OptionId::Help,
        names: &["-h", "--help"],
        value: OptionValue::Flag,
        commands: HELP_COMMANDS,
        root: true,
        version: false,
        help_line: "  -h, --help  Print help\n",
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CommandSpec {
    pub(crate) id: CommandId,
    pub(crate) path: &'static [&'static str],
    pub(crate) root_usage: &'static str,
    pub(crate) summary: &'static str,
    pub(crate) help: Option<&'static str>,
    pub(crate) help_options: &'static [OptionId],
}

pub(crate) const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        id: CommandId::Convert,
        path: &["convert"],
        root_usage: "",
        summary: "Convert an AsciiDoc document to HTML",
        help: Some(
            "Usage:\n  adocweave convert [OPTIONS] [FILE]\n\nArguments:\n  FILE  Input file; omit or use - for standard input\n\nOptions:\n@OPTIONS@\nExample:\n  adocweave convert --complete manual.adoc\n",
        ),
        help_options: &[
            OptionId::Complete,
            OptionId::Css,
            OptionId::CssUrl,
            OptionId::Include,
            OptionId::NoInclude,
            OptionId::StdinBase,
            OptionId::AllowRoot,
            OptionId::Config,
            OptionId::NoConfig,
            OptionId::Color,
            OptionId::Help,
        ],
    },
    CommandSpec {
        id: CommandId::Preview,
        path: &["preview"],
        root_usage: "",
        summary: "Serve a live document preview",
        help: Some(
            "\
Usage:
  adocweave preview [OPTIONS] FILE

Arguments:
  FILE  AsciiDoc file to preview; standard input and symbolic links are not supported

Options:
@OPTIONS@
Security:
  A non-loopback address requires --allow-external.
  The server does not provide authentication or TLS encryption.

Example:
  adocweave preview --port 8080 manual.adoc
",
        ),
        help_options: &[
            OptionId::Bind,
            OptionId::Port,
            OptionId::Debounce,
            OptionId::AllowExternal,
            OptionId::Include,
            OptionId::NoInclude,
            OptionId::AllowRoot,
            OptionId::Css,
            OptionId::CssUrl,
            OptionId::Config,
            OptionId::NoConfig,
            OptionId::Color,
            OptionId::Help,
        ],
    },
    CommandSpec {
        id: CommandId::Check,
        path: &["check"],
        root_usage: "",
        summary: "Check AsciiDoc documents",
        help: Some(
            "Usage:\n  adocweave check [OPTIONS] [FILE...]\n\nArguments:\n  FILE  Input file or directory; omit or use - for standard input\n\nOptions:\n@OPTIONS@\nExamples:\n  adocweave check --fail-on warning docs\n  adocweave check --format sarif docs > adocweave.sarif\n  adocweave check --fix docs\n  adocweave check --fix --diff docs\n",
        ),
        help_options: &[
            OptionId::DiagnosticFormat,
            OptionId::FailOn,
            OptionId::Summary,
            OptionId::Fix,
            OptionId::Diff,
            OptionId::EnableRule,
            OptionId::Glob,
            OptionId::ProjectRoot,
            OptionId::Include,
            OptionId::NoInclude,
            OptionId::StdinBase,
            OptionId::AllowRoot,
            OptionId::Config,
            OptionId::NoConfig,
            OptionId::Color,
            OptionId::Help,
        ],
    },
    CommandSpec {
        id: CommandId::Format,
        path: &["format"],
        root_usage: "",
        summary: "Format AsciiDoc documents",
        help: Some(
            "Usage:\n  adocweave format [OPTIONS] [FILE...]\n\nArguments:\n  FILE  Input file or directory; omit or use - for standard input\n\nOptions:\n@OPTIONS@\nExamples:\n  adocweave format --check docs\n  adocweave format --diff manual.adoc\n  adocweave format --write docs\n",
        ),
        help_options: &[
            OptionId::FormatCheck,
            OptionId::Diff,
            OptionId::FormatWrite,
            OptionId::Summary,
            OptionId::Glob,
            OptionId::Include,
            OptionId::NoInclude,
            OptionId::StdinBase,
            OptionId::AllowRoot,
            OptionId::Config,
            OptionId::NoConfig,
            OptionId::Color,
            OptionId::Help,
        ],
    },
    CommandSpec {
        id: CommandId::Symbols,
        path: &["symbols"],
        root_usage: "",
        summary: "Print document symbols as JSON",
        help: Some(
            "Usage:\n  adocweave symbols [OPTIONS] [FILE]\n\nArguments:\n  FILE  Input file; omit or use - for standard input\n\nOptions:\n@OPTIONS@\nExample:\n  adocweave symbols manual.adoc\n",
        ),
        help_options: &[
            OptionId::Include,
            OptionId::NoInclude,
            OptionId::StdinBase,
            OptionId::AllowRoot,
            OptionId::Config,
            OptionId::NoConfig,
            OptionId::Color,
            OptionId::Help,
        ],
    },
    CommandSpec {
        id: CommandId::Rules,
        path: &["rules"],
        root_usage: "",
        summary: "List diagnostic rules",
        help: Some(
            "Usage:\n  adocweave rules [OPTIONS]\n\nOptions:\n@OPTIONS@\nExamples:\n  adocweave rules\n  adocweave rules --format json\n",
        ),
        help_options: &[OptionId::RuleFormat, OptionId::Help],
    },
    CommandSpec {
        id: CommandId::ConfigShow,
        path: &["config", "show"],
        root_usage: "",
        summary: "Print the resolved project configuration as JSON",
        help: Some(
            "Usage:\n  adocweave config show [OPTIONS]\n\nOptions:\n@OPTIONS@\nExample:\n  adocweave config show\n",
        ),
        help_options: &[OptionId::Config, OptionId::NoConfig, OptionId::Help],
    },
    CommandSpec {
        id: CommandId::Completion,
        path: &["completion"],
        root_usage: " SHELL",
        summary: "Print a shell completion script",
        help: Some(
            "Usage:\n  adocweave completion SHELL\n\nArguments:\n  SHELL  One of bash, zsh, fish, or powershell\n\nOptions:\n@OPTIONS@\nExample:\n  adocweave completion bash\n",
        ),
        help_options: &[OptionId::Help],
    },
    CommandSpec {
        id: CommandId::Help,
        path: &["help"],
        root_usage: "",
        summary: "Print root help",
        help: None,
        help_options: &[],
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LookupError<'a> {
    UnknownCommand(&'a str),
    MissingSubcommand(&'static str),
    UnknownSubcommand {
        parent: &'static str,
        value: &'a str,
    },
}

pub(crate) fn lookup(tokens: &[String]) -> Result<(CommandId, usize), LookupError<'_>> {
    validate_public_model();
    lookup_in(COMMANDS, tokens)
}

fn lookup_in<'a>(
    commands: &[CommandSpec],
    tokens: &'a [String],
) -> Result<(CommandId, usize), LookupError<'a>> {
    let Some(first) = tokens.first() else {
        return Err(LookupError::UnknownCommand(""));
    };
    let candidates = commands
        .iter()
        .filter(|spec| spec.path.first() == Some(&first.as_str()))
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Err(LookupError::UnknownCommand(first));
    }
    if let Some(spec) = candidates
        .iter()
        .filter(|spec| {
            tokens.len() >= spec.path.len()
                && spec
                    .path
                    .iter()
                    .zip(tokens)
                    .all(|(expected, actual)| expected == actual)
        })
        .max_by_key(|spec| spec.path.len())
    {
        return Ok((spec.id, spec.path.len()));
    }
    let parent = candidates[0].path[0];
    let Some(value) = tokens.get(1) else {
        return Err(LookupError::MissingSubcommand(parent));
    };
    Err(LookupError::UnknownSubcommand { parent, value })
}

pub(crate) fn spec(id: CommandId) -> &'static CommandSpec {
    COMMANDS
        .iter()
        .find(|spec| spec.id == id)
        .expect("every CommandId has a CommandSpec")
}

pub(crate) fn option_by_name(name: &str) -> Option<&'static OptionSpec> {
    validate_public_model();
    OPTIONS.iter().find(|option| option.names.contains(&name))
}

pub(crate) fn option_for_command(command: CommandId, name: &str) -> Option<&'static OptionSpec> {
    validate_public_model();
    OPTIONS
        .iter()
        .find(|option| option.names.contains(&name) && option.applies_to(command))
}

pub(crate) fn root_option(name: &str) -> Option<&'static OptionSpec> {
    validate_public_model();
    OPTIONS
        .iter()
        .find(|option| option.names.contains(&name) && option.root)
}

pub(crate) fn version_option(name: &str) -> Option<&'static OptionSpec> {
    validate_public_model();
    OPTIONS
        .iter()
        .find(|option| option.names.contains(&name) && option.version)
}

pub(crate) fn option(id: OptionId) -> &'static OptionSpec {
    OPTIONS
        .iter()
        .find(|option| option.id == id)
        .expect("every OptionId has an OptionSpec")
}

pub(crate) fn options_for_command(command: CommandId) -> impl Iterator<Item = &'static OptionSpec> {
    OPTIONS
        .iter()
        .filter(move |option| option.applies_to(command))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompletionGroup {
    pub(crate) parent: Vec<&'static str>,
    pub(crate) children: Vec<&'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompletionTree {
    pub(crate) roots: Vec<&'static str>,
    pub(crate) nested: Vec<CompletionGroup>,
    pub(crate) commands: Vec<(CommandId, Vec<&'static str>)>,
}

pub(crate) fn completion_tree() -> CompletionTree {
    validate_public_model();
    completion_tree_from(COMMANDS)
}

fn completion_tree_from(commands: &[CommandSpec]) -> CompletionTree {
    let mut roots = Vec::new();
    let mut nested: Vec<CompletionGroup> = Vec::new();
    for command in commands {
        let root = command.path[0];
        if !roots.contains(&root) {
            roots.push(root);
        }
        for depth in 1..command.path.len() {
            let parent = command.path[..depth].to_vec();
            let child = command.path[depth];
            if let Some(group) = nested.iter_mut().find(|group| group.parent == parent) {
                if !group.children.contains(&child) {
                    group.children.push(child);
                }
            } else {
                nested.push(CompletionGroup {
                    parent,
                    children: vec![child],
                });
            }
        }
    }
    CompletionTree {
        roots,
        nested,
        commands: commands
            .iter()
            .map(|command| (command.id, command.path.to_vec()))
            .collect(),
    }
}

fn valid_path_token(token: &str) -> bool {
    let mut bytes = token.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_option_name(name: &str) -> bool {
    if let Some(long) = name.strip_prefix("--") {
        return valid_path_token(long);
    }
    let bytes = name.as_bytes();
    bytes.len() == 2
        && bytes[0] == b'-'
        && (bytes[1].is_ascii_lowercase() || bytes[1].is_ascii_uppercase())
}

fn valid_metavar(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte == b'-')
}

fn validate_public_model() {
    validate_model(COMMANDS).expect("command model must be unambiguous");
    validate_options(OPTIONS, COMMANDS).expect("option model must be complete and unambiguous");
}

#[cfg(test)]
pub(crate) fn completion_tree_for_tests(commands: &[CommandSpec]) -> CompletionTree {
    if let Err(error) = validate_model(commands) {
        panic!("invalid test command model: {error}");
    }
    completion_tree_from(commands)
}

fn validate_model(commands: &[CommandSpec]) -> Result<(), &'static str> {
    for (index, command) in commands.iter().enumerate() {
        if command.path.is_empty() || command.path.iter().any(|token| !valid_path_token(token)) {
            return Err("command path tokens must match ^[a-z0-9][a-z0-9-]*$");
        }
        if command.summary.is_empty() {
            return Err("command summaries must not be empty");
        }
        if command.help.is_none() && !command.help_options.is_empty() {
            return Err("commands without help cannot select help options");
        }
        if command.help.is_some_and(|help| {
            help.matches("@OPTIONS@").count() != usize::from(!command.help_options.is_empty())
        }) {
            return Err("command help must contain one option placeholder exactly when needed");
        }
        for other in &commands[index + 1..] {
            if command.id == other.id {
                return Err("command ids must be unique");
            }
            let shared = command.path.len().min(other.path.len());
            if command.path[..shared] == other.path[..shared] {
                return Err("command paths must not duplicate or prefix another command");
            }
        }
    }
    Ok(())
}

fn validate_options(options: &[OptionSpec], commands: &[CommandSpec]) -> Result<(), &'static str> {
    for (index, option) in options.iter().enumerate() {
        if option.names.is_empty()
            || option.names.iter().any(|name| !valid_option_name(name))
            || !option.names.iter().any(|name| name.starts_with("--"))
        {
            return Err("options must have valid unique names including one long name");
        }
        if option.help_line.is_empty()
            || !option.help_line.ends_with('\n')
            || !option
                .names
                .iter()
                .all(|name| option.help_line.contains(name))
        {
            return Err("help lines must contain every option name and end with a newline");
        }
        match option.value {
            OptionValue::Flag => {}
            OptionValue::Required {
                metavar,
                missing,
                candidates,
            } => {
                if !valid_metavar(metavar)
                    || missing.is_empty()
                    || !option.help_line.contains(metavar)
                {
                    return Err("valued options require a valid metavar and missing-value label");
                }
                for (candidate_index, candidate) in candidates.iter().enumerate() {
                    if !valid_path_token(candidate)
                        || candidates[candidate_index + 1..].contains(candidate)
                    {
                        return Err("option value candidates must be safe and unique");
                    }
                }
            }
        }
        for (command_index, command) in option.commands.iter().enumerate() {
            if !commands
                .iter()
                .any(|spec| spec.id == *command && spec.help.is_some())
                || option.commands[command_index + 1..].contains(command)
            {
                return Err(
                    "option command applicability must name documented commands and be unique",
                );
            }
        }
        if !option.root && !option.version && option.commands.is_empty() {
            return Err("every option must have at least one application scope");
        }
        for other in &options[index + 1..] {
            if option.id == other.id {
                return Err("option ids must be unique");
            }
            let overlapping_scope = option
                .commands
                .iter()
                .any(|command| other.commands.contains(command))
                || (option.root && other.root)
                || (option.version && other.version);
            if overlapping_scope && option.names.iter().any(|name| other.names.contains(name)) {
                return Err("option names must be unique");
            }
        }
    }
    for command in commands {
        for (index, option_id) in command.help_options.iter().enumerate() {
            if command.help_options[index + 1..].contains(option_id) {
                return Err("command help option ids must be unique");
            }
            let Some(option) = options.iter().find(|option| option.id == *option_id) else {
                return Err("command help options must exist");
            };
            let line = option.help_line;
            if !option.applies_to(command.id) {
                return Err("command help options must apply to the command");
            }
            if !line.contains(option.canonical_name()) {
                return Err("command help lines must contain the canonical option name");
            }
            if option
                .metavar()
                .is_some_and(|metavar| !line.contains(metavar))
            {
                return Err("command help lines must contain the option metavar");
            }
            if !line.ends_with('\n') {
                return Err("command help lines must end with a newline");
            }
        }
    }
    Ok(())
}

pub(crate) fn command_help(id: CommandId) -> Option<String> {
    validate_public_model();
    let command = spec(id);
    command.help.map(|template| {
        let options = command
            .help_options
            .iter()
            .map(|id| option(*id).help_line)
            .collect::<String>();
        template.replace("@OPTIONS@", &options)
    })
}

pub(crate) fn root_help() -> String {
    validate_public_model();
    let mut commands = String::new();
    for spec in COMMANDS {
        let path = format!("{}{}", spec.path.join(" "), spec.root_usage);
        if spec.path.len() == 1 {
            commands.push_str(&format!("  {path:<7}  {}\n", spec.summary));
        } else {
            commands.push_str(&format!("  {path}  {}\n", spec.summary));
        }
    }
    let options = OPTIONS
        .iter()
        .filter(|option| option.root)
        .map(|option| option.help_line)
        .collect::<String>();
    format!(
        "\
AdocWeave command-line interface

Usage:
  adocweave <COMMAND> [ARGUMENTS]

Commands:
{commands}
Options:
{options}\
"
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn command_model_has_unique_ids_and_paths() {
        assert_eq!(COMMANDS.len(), 9);
        assert_eq!(
            COMMANDS.iter().map(|spec| spec.id).collect::<BTreeSet<_>>(),
            BTreeSet::from([
                CommandId::Convert,
                CommandId::Preview,
                CommandId::Check,
                CommandId::Format,
                CommandId::Symbols,
                CommandId::Rules,
                CommandId::ConfigShow,
                CommandId::Completion,
                CommandId::Help,
            ])
        );
        assert_eq!(
            COMMANDS
                .iter()
                .map(|spec| spec.path.join(" "))
                .collect::<BTreeSet<_>>()
                .len(),
            COMMANDS.len()
        );
        assert!(COMMANDS.iter().all(|spec| {
            !spec.path.is_empty()
                && spec.path.iter().all(|token| !token.is_empty())
                && !spec.summary.is_empty()
        }));
        assert_eq!(
            COMMANDS.iter().filter(|spec| spec.help.is_none()).count(),
            1
        );
        assert_eq!(validate_model(COMMANDS), Ok(()));
    }

    #[test]
    fn option_model_is_complete_and_has_exact_command_applicability() {
        assert_eq!(validate_options(OPTIONS, COMMANDS), Ok(()));
        assert_eq!(OPTIONS.len(), 28);
        assert_eq!(
            OPTIONS
                .iter()
                .map(|option| option.id)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                OptionId::DiagnosticFormat,
                OptionId::RuleFormat,
                OptionId::Json,
                OptionId::FailOn,
                OptionId::Summary,
                OptionId::Fix,
                OptionId::Config,
                OptionId::NoConfig,
                OptionId::EnableRule,
                OptionId::FormatCheck,
                OptionId::FormatWrite,
                OptionId::Diff,
                OptionId::Glob,
                OptionId::Color,
                OptionId::Include,
                OptionId::NoInclude,
                OptionId::StdinBase,
                OptionId::AllowRoot,
                OptionId::ProjectRoot,
                OptionId::Complete,
                OptionId::Css,
                OptionId::CssUrl,
                OptionId::Bind,
                OptionId::Port,
                OptionId::Debounce,
                OptionId::AllowExternal,
                OptionId::Version,
                OptionId::Help,
            ])
        );

        let ids = |command| {
            options_for_command(command)
                .map(|option| option.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(CommandId::Convert),
            [
                OptionId::Config,
                OptionId::NoConfig,
                OptionId::Color,
                OptionId::Include,
                OptionId::NoInclude,
                OptionId::StdinBase,
                OptionId::AllowRoot,
                OptionId::Complete,
                OptionId::Css,
                OptionId::CssUrl,
                OptionId::Help,
            ]
        );
        assert_eq!(
            ids(CommandId::Preview),
            [
                OptionId::Config,
                OptionId::NoConfig,
                OptionId::Color,
                OptionId::Include,
                OptionId::NoInclude,
                OptionId::AllowRoot,
                OptionId::Css,
                OptionId::CssUrl,
                OptionId::Bind,
                OptionId::Port,
                OptionId::Debounce,
                OptionId::AllowExternal,
                OptionId::Help,
            ]
        );
        assert_eq!(
            ids(CommandId::Check),
            [
                OptionId::DiagnosticFormat,
                OptionId::FailOn,
                OptionId::Summary,
                OptionId::Fix,
                OptionId::Config,
                OptionId::NoConfig,
                OptionId::EnableRule,
                OptionId::Diff,
                OptionId::Glob,
                OptionId::Color,
                OptionId::Include,
                OptionId::NoInclude,
                OptionId::StdinBase,
                OptionId::AllowRoot,
                OptionId::ProjectRoot,
                OptionId::Help,
            ]
        );
        assert_eq!(
            ids(CommandId::Format),
            [
                OptionId::Summary,
                OptionId::Config,
                OptionId::NoConfig,
                OptionId::FormatCheck,
                OptionId::FormatWrite,
                OptionId::Diff,
                OptionId::Glob,
                OptionId::Color,
                OptionId::Include,
                OptionId::NoInclude,
                OptionId::StdinBase,
                OptionId::AllowRoot,
                OptionId::Help,
            ]
        );
        assert_eq!(
            ids(CommandId::Symbols),
            [
                OptionId::Config,
                OptionId::NoConfig,
                OptionId::Color,
                OptionId::Include,
                OptionId::NoInclude,
                OptionId::StdinBase,
                OptionId::AllowRoot,
                OptionId::Help,
            ]
        );
        assert_eq!(
            ids(CommandId::Rules),
            [OptionId::RuleFormat, OptionId::Help]
        );
        assert_eq!(
            ids(CommandId::ConfigShow),
            [OptionId::Config, OptionId::NoConfig, OptionId::Help]
        );
        assert_eq!(ids(CommandId::Completion), [OptionId::Help]);
        assert!(ids(CommandId::Help).is_empty());
        assert_eq!(
            OPTIONS
                .iter()
                .filter(|option| option.root)
                .map(|option| option.id)
                .collect::<Vec<_>>(),
            [OptionId::Version, OptionId::Help]
        );
        assert_eq!(
            OPTIONS
                .iter()
                .filter(|option| option.version)
                .map(|option| option.id)
                .collect::<Vec<_>>(),
            [OptionId::Json]
        );
    }

    #[test]
    fn value_candidates_are_typed_and_complete() {
        assert_eq!(
            option(OptionId::DiagnosticFormat).candidates(),
            DIAGNOSTIC_FORMATS
        );
        assert_eq!(option(OptionId::FailOn).candidates(), FAILURE_LEVELS);
        assert_eq!(option(OptionId::Color).candidates(), COLOR_CHOICES);
        assert!(OPTIONS.iter().all(|option| {
            matches!(option.value, OptionValue::Required { .. }) == option.metavar().is_some()
        }));
    }

    #[test]
    fn option_validation_rejects_contract_mutations() {
        let mut duplicate_name = OPTIONS.to_vec();
        duplicate_name[1].names = duplicate_name[0].names;
        duplicate_name[1].commands = duplicate_name[0].commands;
        assert!(validate_options(&duplicate_name, COMMANDS).is_err());

        let mut unsafe_candidate = OPTIONS.to_vec();
        unsafe_candidate[0].value = OptionValue::Required {
            metavar: "FORMAT",
            missing: "a value",
            candidates: &["shell value"],
        };
        assert!(validate_options(&unsafe_candidate, COMMANDS).is_err());

        let mut utility_scope = OPTIONS.to_vec();
        utility_scope[0].commands = &[CommandId::Completion];
        assert!(validate_options(&utility_scope, COMMANDS).is_err());

        let mut missing_help_line = OPTIONS.to_vec();
        let help = missing_help_line
            .iter_mut()
            .find(|option| option.id == OptionId::Help)
            .expect("help option");
        help.help_line = "";
        assert!(validate_options(&missing_help_line, COMMANDS).is_err());
    }

    #[test]
    fn nested_command_paths_are_resolved_from_the_model() {
        let tokens = ["config", "show"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(lookup(&tokens), Ok((CommandId::ConfigShow, 2)));
        assert_eq!(
            lookup(&["config".to_owned()]),
            Err(LookupError::MissingSubcommand("config"))
        );
    }

    #[test]
    fn lookup_uses_arbitrary_length_paths_and_rejects_prefix_ambiguity() {
        const DEEP: &[CommandSpec] = &[CommandSpec {
            id: CommandId::ConfigShow,
            path: &["config", "profile", "show"],
            root_usage: "",
            summary: "show profile",
            help: None,
            help_options: &[],
        }];
        let tokens = ["config", "profile", "show", "input"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(lookup_in(DEEP, &tokens), Ok((CommandId::ConfigShow, 3)));

        let ambiguous = [
            CommandSpec {
                id: CommandId::ConfigShow,
                path: &["config"],
                root_usage: "",
                summary: "config",
                help: None,
                help_options: &[],
            },
            CommandSpec {
                id: CommandId::Help,
                path: &["config", "show"],
                root_usage: "",
                summary: "show",
                help: None,
                help_options: &[],
            },
        ];
        assert!(validate_model(&ambiguous).is_err());
    }

    #[test]
    fn completion_tree_is_derived_from_command_paths() {
        let tree = completion_tree();
        assert_eq!(
            tree.roots,
            [
                "convert",
                "preview",
                "check",
                "format",
                "symbols",
                "rules",
                "config",
                "completion",
                "help",
            ]
        );
        assert_eq!(
            tree.nested,
            [CompletionGroup {
                parent: vec!["config"],
                children: vec!["show"],
            }]
        );
    }

    #[test]
    fn unsafe_shell_tokens_are_rejected() {
        for path in [
            &["Config"][..],
            &["bad_name"][..],
            &["bad;name"][..],
            &["-leading"][..],
            &["日本語"][..],
        ] {
            let commands = [CommandSpec {
                id: CommandId::Help,
                path,
                root_usage: "",
                summary: "unsafe",
                help: None,
                help_options: &[],
            }];
            assert!(validate_model(&commands).is_err(), "{path:?}");
        }
    }

    #[test]
    fn generated_help_matches_the_public_snapshots() {
        assert_eq!(
            root_help(),
            include_str!("../../tests/snapshots/help-root.txt")
        );
        let mut command_help = String::new();
        for command in COMMANDS.iter().filter(|command| command.help.is_some()) {
            command_help.push_str(&format!("=== {} ===\n", command.path.join(" ")));
            command_help.push_str(
                &super::command_help(command.id).expect("filtered command has generated help"),
            );
        }
        assert_eq!(
            command_help,
            include_str!("../../tests/snapshots/help-commands.txt")
        );
    }
}
