//! Coloring code by the language it is written in.
//!
//! The backend says which run of text is code and what language the author
//! named for it. What the parts of that code look like is decided here, in the
//! same palette as the rest of the page: a comment, a string, and a keyword are
//! roles like any other, so a reader who changed the palette sees code change
//! with it.
//!
//! The rules below read what a reader looks for while scanning a listing:
//! where the comments are, where the text is, and where a name is introduced.
//! They are not a parser, and a language they do not know keeps the one color
//! a code block already has.

mod languages;

use std::collections::BTreeMap;

use adocweave_core::output::terminal::TerminalDocument;

use crate::theme::CodeRole;
use languages::Language;

/// One piece of a code line, and what that piece is.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Token {
    pub(crate) text: String,
    pub(crate) role: CodeRole,
}

/// The colored pieces of every code line of a document, by line.
///
/// A line that holds no code, or code in a language these rules do not know,
/// is absent: it is shown as the code it already is.
#[derive(Debug, Default)]
pub(crate) struct CodeColors {
    lines: BTreeMap<usize, Vec<Token>>,
}

impl CodeColors {
    pub(crate) fn line(&self, index: usize) -> Option<&[Token]> {
        self.lines.get(&index).map(Vec::as_slice)
    }
}

/// Reads every block of code in `document`.
///
/// A block is read as a whole rather than line by line, so a comment that runs
/// across lines keeps its color to the end of it.
pub(crate) fn code_colors(document: &TerminalDocument) -> CodeColors {
    let mut colors = CodeColors::default();
    let mut block: Vec<(usize, &str)> = Vec::new();
    let mut language: Option<&str> = None;
    for (index, line) in document.lines.iter().enumerate() {
        let code = line
            .spans
            .iter()
            .find_map(|span| Some((span.language.as_deref()?, span.text.as_str())));
        match code {
            Some((found, text)) if language == Some(found) => block.push((index, text)),
            Some((found, text)) => {
                read_block(language, &block, &mut colors);
                language = Some(found);
                block = vec![(index, text)];
            }
            None => {
                read_block(language, &block, &mut colors);
                language = None;
                block.clear();
            }
        }
    }
    read_block(language, &block, &mut colors);
    colors
}

fn read_block(language: Option<&str>, block: &[(usize, &str)], colors: &mut CodeColors) {
    let Some(language) = language.and_then(languages::find) else {
        return;
    };
    let mut scanner = Scanner::new(language);
    for (index, text) in block {
        let tokens = scanner.line(text);
        if tokens.iter().any(|token| token.role != CodeRole::Plain) {
            colors.lines.insert(*index, tokens);
        }
    }
}

/// The keywords after which a word is the name of what is being declared.
///
/// A reader scanning a listing looks for where something is introduced, so
/// the name beside one of these words is worth telling from a name that is
/// merely used.
const DECLARING_KEYWORDS: &[&str] = &[
    "class",
    "def",
    "enum",
    "fn",
    "func",
    "function",
    "impl",
    "interface",
    "module",
    "namespace",
    "package",
    "struct",
    "trait",
    "type",
    "union",
];

/// Reads the lines of one block, keeping what runs across them.
struct Scanner {
    language: &'static Language,
    /// Whether the line begins inside a comment opened on an earlier line.
    in_block_comment: bool,
    /// The last word read on this line, which says whether the next one is
    /// the name of something being declared.
    previous_word: Option<String>,
}

impl Scanner {
    const fn new(language: &'static Language) -> Self {
        Self {
            language,
            in_block_comment: false,
            previous_word: None,
        }
    }

    /// Breaks one line into its pieces. Every piece of the line is kept, in
    /// order, so the text a reader sees is exactly the text that was written.
    fn line(&mut self, line: &str) -> Vec<Token> {
        let mut tokens = Tokens::default();
        self.previous_word = None;
        let characters: Vec<char> = line.chars().collect();
        let mut index = 0;
        while index < characters.len() {
            if self.in_block_comment {
                index = self.continue_block_comment(&characters, index, &mut tokens);
                continue;
            }
            if let Some(next) = self.start_of_comment(&characters, index, &mut tokens) {
                index = next;
                continue;
            }
            if let Some(next) = self.text(&characters, index, &mut tokens) {
                index = next;
                continue;
            }
            if let Some(next) = number(&characters, index, &mut tokens) {
                index = next;
                continue;
            }
            if let Some(next) = self.word(&characters, index, &mut tokens) {
                index = next;
                continue;
            }
            tokens.push(characters[index], CodeRole::Plain);
            index += 1;
        }
        tokens.finish()
    }

    /// Reads to the end of a comment that began on an earlier line.
    fn continue_block_comment(
        &mut self,
        characters: &[char],
        mut index: usize,
        tokens: &mut Tokens,
    ) -> usize {
        let Some((_, close)) = self.language.block_comment else {
            self.in_block_comment = false;
            return index;
        };
        while index < characters.len() {
            if matches(characters, index, close) {
                for offset in 0..close.chars().count() {
                    tokens.push(characters[index + offset], CodeRole::Comment);
                }
                self.in_block_comment = false;
                return index + close.chars().count();
            }
            tokens.push(characters[index], CodeRole::Comment);
            index += 1;
        }
        index
    }

    /// A comment that starts here, whether it ends on this line or later.
    fn start_of_comment(
        &mut self,
        characters: &[char],
        index: usize,
        tokens: &mut Tokens,
    ) -> Option<usize> {
        for marker in self.language.line_comments {
            if matches(characters, index, marker) {
                for character in &characters[index..] {
                    tokens.push(*character, CodeRole::Comment);
                }
                return Some(characters.len());
            }
        }
        let (open, _) = self.language.block_comment?;
        if !matches(characters, index, open) {
            return None;
        }
        for offset in 0..open.chars().count() {
            tokens.push(characters[index + offset], CodeRole::Comment);
        }
        self.in_block_comment = true;
        Some(self.continue_block_comment(characters, index + open.chars().count(), tokens))
    }

    /// Text written inside the code, in whichever quotes the language uses.
    ///
    /// A quotation that is never closed ends with the line. Carrying it further
    /// would color the rest of the block as text on the strength of one typing
    /// mistake.
    fn text(&self, characters: &[char], index: usize, tokens: &mut Tokens) -> Option<usize> {
        let quote = *self
            .language
            .quotes
            .iter()
            .find(|quote| characters[index] == **quote)?;
        tokens.push(quote, CodeRole::Text);
        let mut position = index + 1;
        while position < characters.len() {
            let character = characters[position];
            tokens.push(character, CodeRole::Text);
            position += 1;
            if character == '\\' && position < characters.len() {
                tokens.push(characters[position], CodeRole::Text);
                position += 1;
                continue;
            }
            if character == quote {
                break;
            }
        }
        Some(position)
    }

    /// A word, which is a keyword, the name of what is being declared, the
    /// name of something being called, or ordinary code.
    fn word(&mut self, characters: &[char], index: usize, tokens: &mut Tokens) -> Option<usize> {
        if !is_word_start(characters[index]) {
            return None;
        }
        let mut end = index;
        while end < characters.len() && is_word(characters[end]) {
            end += 1;
        }
        let word: String = characters[index..end].iter().collect();
        let role = if self.language.keywords.contains(&word.as_str()) {
            CodeRole::Keyword
        } else if self.declares_a_name() {
            CodeRole::Name
        } else if called(characters, end) {
            CodeRole::Function
        } else {
            CodeRole::Plain
        };
        for character in &characters[index..end] {
            tokens.push(*character, role);
        }
        self.previous_word = Some(word);
        Some(end)
    }

    /// Whether the word just read stands where a declared name stands.
    fn declares_a_name(&self) -> bool {
        self.previous_word.as_deref().is_some_and(|previous| {
            DECLARING_KEYWORDS.contains(&previous) && self.language.keywords.contains(&previous)
        })
    }
}

/// A number written literally in the code.
fn number(characters: &[char], index: usize, tokens: &mut Tokens) -> Option<usize> {
    if !characters[index].is_ascii_digit() {
        return None;
    }
    // A digit inside a word is part of that word, not a number of its own.
    if index > 0 && is_word(characters[index - 1]) {
        return None;
    }
    let mut end = index;
    while end < characters.len()
        && (characters[end].is_ascii_alphanumeric()
            || characters[end] == '.'
            || characters[end] == '_')
    {
        end += 1;
    }
    for character in &characters[index..end] {
        tokens.push(*character, CodeRole::Value);
    }
    Some(end)
}

/// Whether the word that ends at `index` is being called, which is the one
/// thing a listing shows about a name without any knowledge of the language.
fn called(characters: &[char], index: usize) -> bool {
    characters
        .get(index)
        .is_some_and(|character| *character == '(' || *character == '!')
}

fn is_word_start(character: char) -> bool {
    character.is_alphabetic() || character == '_'
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Whether `marker` stands at `index`.
fn matches(characters: &[char], index: usize, marker: &str) -> bool {
    marker
        .chars()
        .enumerate()
        .all(|(offset, expected)| characters.get(index + offset) == Some(&expected))
}

/// The pieces of one line, with neighbours of the same kind joined.
#[derive(Default)]
struct Tokens {
    tokens: Vec<Token>,
}

impl Tokens {
    fn push(&mut self, character: char, role: CodeRole) {
        match self.tokens.last_mut() {
            Some(last) if last.role == role => last.text.push(character),
            _ => self.tokens.push(Token {
                text: character.to_string(),
                role,
            }),
        }
    }

    fn finish(self) -> Vec<Token> {
        self.tokens
    }
}

#[cfg(test)]
mod tests {
    use adocweave_core::output::terminal::{
        TerminalLine, TerminalRole, TerminalSpan, TerminalStyle,
    };

    use super::*;

    fn code_line(text: &str, language: Option<&str>) -> TerminalLine {
        TerminalLine {
            spans: vec![TerminalSpan {
                language: language.map(str::to_owned),
                ..TerminalSpan::new(text, TerminalStyle::of(TerminalRole::Code))
            }],
        }
    }

    fn colors(lines: &[(&str, Option<&str>)]) -> CodeColors {
        let document = TerminalDocument {
            lines: lines
                .iter()
                .map(|(text, language)| code_line(text, *language))
                .collect(),
        };
        code_colors(&document)
    }

    fn roles(tokens: &[Token]) -> Vec<(&str, CodeRole)> {
        tokens
            .iter()
            .map(|token| (token.text.as_str(), token.role))
            .collect()
    }

    #[test]
    fn a_keyword_a_string_and_a_comment_are_told_apart() {
        let colors = colors(&[("let value = \"text\"; // why", Some("rust"))]);
        let tokens = colors.line(0).expect("a colored line");

        assert_eq!(
            roles(tokens),
            vec![
                ("let", CodeRole::Keyword),
                (" value = ", CodeRole::Plain),
                ("\"text\"", CodeRole::Text),
                ("; ", CodeRole::Plain),
                ("// why", CodeRole::Comment),
            ]
        );
    }

    /// Every piece of a line is kept, in order, whatever the roles are.
    #[test]
    fn the_pieces_of_a_line_add_up_to_the_line() {
        let text = "fn main() { let value = 1_000; }";
        let colors = colors(&[(text, Some("rust"))]);
        let joined: String = colors
            .line(0)
            .expect("a colored line")
            .iter()
            .map(|token| token.text.as_str())
            .collect();

        assert_eq!(joined, text);
    }

    #[test]
    fn a_name_that_is_called_is_told_from_one_that_is_not() {
        let colors = colors(&[("value = compute(other)", Some("rust"))]);
        let tokens = roles(colors.line(0).expect("a colored line"));

        assert!(
            tokens.contains(&("compute", CodeRole::Function)),
            "{tokens:?}"
        );
        assert!(
            tokens
                .iter()
                .any(|(text, role)| text.contains("other") && *role == CodeRole::Plain)
        );
    }

    #[test]
    fn a_number_is_told_from_a_name_that_holds_digits() {
        let colors = colors(&[("value2 = 42 + 3.5", Some("rust"))]);
        let tokens = roles(colors.line(0).expect("a colored line"));

        assert!(tokens.contains(&("42", CodeRole::Value)), "{tokens:?}");
        assert!(tokens.contains(&("3.5", CodeRole::Value)), "{tokens:?}");
        assert!(
            !tokens
                .iter()
                .any(|(text, role)| *text == "2" && *role == CodeRole::Value),
            "{tokens:?}"
        );
    }

    /// A comment that runs across lines keeps its color, because the block is
    /// read as a whole.
    #[test]
    fn a_comment_that_spans_lines_stays_a_comment() {
        let colors = colors(&[
            ("/* start", Some("rust")),
            ("still inside", Some("rust")),
            ("end */ let", Some("rust")),
        ]);

        assert_eq!(
            roles(colors.line(1).expect("a colored line")),
            vec![("still inside", CodeRole::Comment)]
        );
        assert_eq!(
            roles(colors.line(2).expect("a colored line")),
            vec![
                ("end */", CodeRole::Comment),
                (" ", CodeRole::Plain),
                ("let", CodeRole::Keyword),
            ]
        );
    }

    /// Two blocks are read separately, so a comment left open in one does not
    /// color the next.
    #[test]
    fn a_comment_left_open_does_not_reach_the_next_block() {
        let colors = colors(&[
            ("/* start", Some("rust")),
            ("", None),
            ("let value = 1;", Some("rust")),
        ]);
        let second = roles(colors.line(2).expect("a colored line"));

        assert!(second.contains(&("let", CodeRole::Keyword)), "{second:?}");
    }

    /// A quotation that is never closed ends with its line, so one typing
    /// mistake does not color the rest of the block.
    #[test]
    fn a_quotation_that_is_never_closed_ends_with_its_line() {
        let colors = colors(&[
            ("let value = \"open", Some("rust")),
            ("let other = 1;", Some("rust")),
        ]);
        let second = roles(colors.line(1).expect("a colored line"));

        assert!(second.contains(&("let", CodeRole::Keyword)), "{second:?}");
    }

    /// The name of what a line declares is told from a name it merely uses.
    #[test]
    fn a_declared_name_is_told_from_one_that_is_used() {
        let colors = colors(&[("fn compute() { other() }", Some("rust"))]);
        let tokens = roles(colors.line(0).expect("a colored line"));

        assert!(tokens.contains(&("compute", CodeRole::Name)), "{tokens:?}");
        assert!(
            tokens.contains(&("other", CodeRole::Function)),
            "{tokens:?}"
        );
    }

    #[test]
    fn a_language_these_rules_do_not_know_is_left_as_it_is() {
        assert!(colors(&[("plain code", Some("no-such"))]).line(0).is_none());
    }

    #[test]
    fn text_that_is_not_code_is_never_colored() {
        assert!(colors(&[("ordinary text", None)]).line(0).is_none());
    }

    /// Each language is read by its own rules: a hash opens a comment in one
    /// and nothing in another.
    #[test]
    fn each_language_is_read_by_its_own_rules() {
        let python = colors(&[("value = 1  # why", Some("python"))]);
        let rust = colors(&[("value = 1;  # why", Some("rust"))]);

        assert!(
            roles(python.line(0).expect("a colored line")).contains(&("# why", CodeRole::Comment))
        );
        assert!(
            !roles(rust.line(0).expect("a colored line"))
                .iter()
                .any(|(_, role)| *role == CodeRole::Comment)
        );
    }
}
