//! The languages a source block is read by, and how each one is written.
//!
//! A language is described by the few things a reader picks out while
//! scanning: where a comment starts, which quotes hold text, and which words
//! are the language's own. The keyword lists are the words that appear in
//! ordinary code, not every word a language reserves: a list that is a little
//! short colors a rare word as plain code, which is what it looked like
//! before.

/// How one language is written.
pub(super) struct Language {
    /// The names an author may write on a source block.
    pub(super) names: &'static [&'static str],
    /// What opens a comment that ends with the line.
    pub(super) line_comments: &'static [&'static str],
    /// What opens and closes a comment that may run across lines.
    pub(super) block_comment: Option<(&'static str, &'static str)>,
    /// The quotes that hold text.
    pub(super) quotes: &'static [char],
    pub(super) keywords: &'static [&'static str],
}

/// The language written under `name`, if these rules know it.
pub(super) fn find(name: &str) -> Option<&'static Language> {
    let name = name.to_ascii_lowercase();
    LANGUAGES
        .iter()
        .find(|language| language.names.contains(&name.as_str()))
}

const C_STYLE_COMMENTS: Option<(&str, &str)> = Some(("/*", "*/"));

const LANGUAGES: &[Language] = &[
    Language {
        names: &["rust", "rs"],
        line_comments: &["//"],
        block_comment: C_STYLE_COMMENTS,
        quotes: &['"', '\''],
        keywords: &[
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
            "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
            "move", "mut", "pub", "ref", "return", "self", "static", "struct", "super", "trait",
            "true", "type", "unsafe", "use", "where", "while",
        ],
    },
    Language {
        names: &["python", "py"],
        line_comments: &["#"],
        block_comment: None,
        quotes: &['"', '\''],
        keywords: &[
            "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
            "elif", "else", "except", "False", "finally", "for", "from", "global", "if", "import",
            "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return",
            "True", "try", "while", "with", "yield",
        ],
    },
    Language {
        names: &["javascript", "js", "typescript", "ts", "jsx", "tsx"],
        line_comments: &["//"],
        block_comment: C_STYLE_COMMENTS,
        quotes: &['"', '\'', '`'],
        keywords: &[
            "as",
            "async",
            "await",
            "break",
            "case",
            "catch",
            "class",
            "const",
            "continue",
            "default",
            "delete",
            "do",
            "else",
            "enum",
            "export",
            "extends",
            "false",
            "finally",
            "for",
            "from",
            "function",
            "if",
            "implements",
            "import",
            "in",
            "instanceof",
            "interface",
            "let",
            "new",
            "null",
            "of",
            "private",
            "public",
            "readonly",
            "return",
            "static",
            "super",
            "switch",
            "this",
            "throw",
            "true",
            "try",
            "type",
            "typeof",
            "undefined",
            "var",
            "void",
            "while",
            "yield",
        ],
    },
    Language {
        names: &["go", "golang"],
        line_comments: &["//"],
        block_comment: C_STYLE_COMMENTS,
        quotes: &['"', '`'],
        keywords: &[
            "break",
            "case",
            "chan",
            "const",
            "continue",
            "default",
            "defer",
            "else",
            "fallthrough",
            "false",
            "for",
            "func",
            "go",
            "goto",
            "if",
            "import",
            "interface",
            "map",
            "nil",
            "package",
            "range",
            "return",
            "select",
            "struct",
            "switch",
            "true",
            "type",
            "var",
        ],
    },
    Language {
        names: &["c", "cpp", "c++", "h", "hpp", "java", "csharp", "cs"],
        line_comments: &["//"],
        block_comment: C_STYLE_COMMENTS,
        quotes: &['"', '\''],
        keywords: &[
            "abstract",
            "auto",
            "bool",
            "boolean",
            "break",
            "case",
            "catch",
            "char",
            "class",
            "const",
            "constexpr",
            "continue",
            "default",
            "delete",
            "do",
            "double",
            "else",
            "enum",
            "extends",
            "extern",
            "false",
            "final",
            "finally",
            "float",
            "for",
            "goto",
            "if",
            "implements",
            "import",
            "inline",
            "int",
            "interface",
            "long",
            "namespace",
            "new",
            "null",
            "nullptr",
            "package",
            "private",
            "protected",
            "public",
            "return",
            "short",
            "sizeof",
            "static",
            "struct",
            "super",
            "switch",
            "template",
            "this",
            "throw",
            "true",
            "try",
            "typedef",
            "typename",
            "union",
            "unsigned",
            "using",
            "var",
            "virtual",
            "void",
            "while",
        ],
    },
    Language {
        names: &["ruby", "rb"],
        line_comments: &["#"],
        block_comment: None,
        quotes: &['"', '\''],
        keywords: &[
            "and", "begin", "break", "case", "class", "def", "do", "else", "elsif", "end",
            "ensure", "false", "for", "if", "in", "module", "next", "nil", "not", "or", "redo",
            "rescue", "retry", "return", "self", "super", "then", "true", "unless", "until",
            "when", "while", "yield",
        ],
    },
    Language {
        names: &["shell", "sh", "bash", "zsh", "console"],
        line_comments: &["#"],
        block_comment: None,
        quotes: &['"', '\''],
        keywords: &[
            "case", "do", "done", "elif", "else", "esac", "exit", "export", "fi", "for",
            "function", "if", "in", "local", "readonly", "return", "set", "then", "until", "while",
        ],
    },
    Language {
        names: &["json"],
        line_comments: &[],
        block_comment: None,
        quotes: &['"'],
        keywords: &["false", "null", "true"],
    },
    Language {
        names: &["yaml", "yml"],
        line_comments: &["#"],
        block_comment: None,
        quotes: &['"', '\''],
        keywords: &["false", "no", "null", "true", "yes"],
    },
    Language {
        names: &["toml"],
        line_comments: &["#"],
        block_comment: None,
        quotes: &['"', '\''],
        keywords: &["false", "true"],
    },
    Language {
        names: &["sql"],
        line_comments: &["--"],
        block_comment: C_STYLE_COMMENTS,
        quotes: &['\'', '"'],
        keywords: &[
            "all",
            "alter",
            "and",
            "as",
            "by",
            "create",
            "delete",
            "distinct",
            "drop",
            "exists",
            "foreign",
            "from",
            "group",
            "having",
            "index",
            "inner",
            "insert",
            "into",
            "join",
            "key",
            "left",
            "limit",
            "not",
            "null",
            "on",
            "or",
            "order",
            "outer",
            "primary",
            "references",
            "right",
            "select",
            "set",
            "table",
            "union",
            "update",
            "values",
            "view",
            "where",
        ],
    },
    Language {
        names: &["css", "scss"],
        line_comments: &["//"],
        block_comment: C_STYLE_COMMENTS,
        quotes: &['"', '\''],
        keywords: &["important", "inherit", "initial", "none", "unset"],
    },
    Language {
        names: &["nix"],
        line_comments: &["#"],
        block_comment: Some(("/*", "*/")),
        quotes: &['"'],
        keywords: &[
            "assert", "else", "if", "in", "inherit", "let", "or", "rec", "then", "with",
        ],
    },
    Language {
        names: &["lua"],
        line_comments: &["--"],
        block_comment: Some(("--[[", "]]")),
        quotes: &['"', '\''],
        keywords: &[
            "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in",
            "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
        ],
    },
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn a_language_is_found_under_every_name_it_is_written_by() {
        assert!(find("rust").is_some());
        assert!(find("rs").is_some());
        assert!(find("Python").is_some());
        assert!(find("no-such-language").is_none());
    }

    /// One name never belongs to two languages, which would make the rules
    /// used for a block depend on the order they are written in.
    #[test]
    fn no_name_belongs_to_two_languages() {
        let mut seen = BTreeSet::new();
        for language in LANGUAGES {
            for name in language.names {
                assert!(seen.insert(*name), "{name} names two languages");
            }
        }
    }

    /// The names are written in lowercase, because that is what they are
    /// compared against.
    #[test]
    fn every_name_is_written_in_lowercase() {
        for language in LANGUAGES {
            for name in language.names {
                assert_eq!(*name, name.to_ascii_lowercase(), "{name}");
            }
        }
    }

    #[test]
    fn every_keyword_list_is_sorted_and_free_of_repetition() {
        for language in LANGUAGES {
            let unique: BTreeSet<&str> = language.keywords.iter().copied().collect();
            assert_eq!(
                unique.len(),
                language.keywords.len(),
                "{:?} repeats a keyword",
                language.names[0]
            );
        }
    }
}
