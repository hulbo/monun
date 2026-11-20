//! The colours of code, as code editors show them: the coloured spans of a text in a language
//! ([`highlight`]), for the code editor, a document's code blocks on screen, in print and in a web
//! page. Python and LaTeX here, SQL in [`sql`], and the languages a Markdown code block names in
//! [`langs`] ([`lang_of`]). Each tokenizer is a pure function that never panics, whatever its
//! input.

mod langs;
mod source;
mod sql;

use std::ops::Range;

pub use langs::lang_of;
pub use source::{Source, source_extensions, source_of};

/// The language a text is coloured as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Python,
    Latex,
    Sql,
    /// Java, Scala and Groovy.
    Java,
    Kotlin,
    /// JavaScript and TypeScript (and Swift, Dart, PHP, read alike).
    Script,
    /// C, C++ and Objective-C.
    C,
    CSharp,
    Go,
    Rust,
    Json,
    /// XML and HTML.
    Markup,
    /// YAML, TOML, INI and properties files.
    Config,
    /// Shell, PowerShell and batch scripts, Dockerfiles and makefiles.
    Shell,
    Css,
    Diff,
    Plain,
}

/// A coloured span's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Keyword,
    String,
    Number,
    Comment,
    Builtin,
}

const PY_KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except",
    "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "match", "case", "nonlocal", "not", "or", "pass", "raise", "return",
    "try", "while", "with", "yield",
];

const PY_BUILTINS: &[&str] = &[
    "abs",
    "all",
    "any",
    "bool",
    "dict",
    "enumerate",
    "filter",
    "float",
    "format",
    "int",
    "isinstance",
    "len",
    "list",
    "map",
    "max",
    "min",
    "print",
    "range",
    "reversed",
    "round",
    "set",
    "sorted",
    "str",
    "sum",
    "tuple",
    "type",
    "zip",
    "xl",
];

/// The coloured spans of `text` (byte ranges on char boundaries, in order, not overlapping).
/// Anything not covered is plain text. Never panics, whatever the input.
pub fn highlight(text: &str, lang: Lang) -> Vec<(Range<usize>, Token)> {
    match lang {
        Lang::Python => python(text),
        Lang::Latex => latex(text),
        Lang::Sql => sql::sql(text),
        Lang::Plain => Vec::new(),
        other => langs::highlight(text, other),
    }
}

/// End (exclusive, in chars) of the Python string literal opening with quote `c` at char `i`.
fn python_string_end(chars: &[(usize, char)], i: usize, c: char) -> usize {
    let triple = chars.get(i + 1).is_some_and(|(_, d)| *d == c) && chars.get(i + 2).is_some_and(|(_, d)| *d == c);
    let mut j = if triple { i + 3 } else { i + 1 };
    loop {
        match chars.get(j) {
            None => break,
            Some((_, '\\')) => j += 2,
            Some((_, '\n')) if !triple => break,
            Some((_, d)) if *d == c => {
                if !triple {
                    j += 1;
                    break;
                }
                if chars.get(j + 1).is_some_and(|(_, d)| *d == c) && chars.get(j + 2).is_some_and(|(_, d)| *d == c) {
                    j += 3;
                    break;
                }
                j += 1;
            }
            Some(_) => j += 1,
        }
    }
    j.min(chars.len())
}

fn python(text: &str) -> Vec<(Range<usize>, Token)> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let end_of = |i: usize| chars.get(i).map(|(b, _)| *b).unwrap_or(text.len());
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&(start, c)) = chars.get(i) {
        if c == '#' {
            let mut j = i;
            while chars.get(j).is_some_and(|(_, c)| *c != '\n') {
                j += 1;
            }
            out.push((start..end_of(j), Token::Comment));
            i = j;
        } else if c == '"' || c == '\'' {
            let j = python_string_end(&chars, i, c);
            out.push((start..end_of(j), Token::String));
            i = j;
        } else if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|(_, d)| d.is_ascii_digit())) {
            let mut j = i + 1;
            while chars.get(j).is_some_and(|(_, d)| d.is_ascii_alphanumeric() || *d == '_' || *d == '.') {
                j += 1;
            }
            out.push((start..end_of(j), Token::Number));
            i = j;
        } else if c.is_alphabetic() || c == '_' {
            let mut j = i + 1;
            while chars.get(j).is_some_and(|(_, d)| d.is_alphanumeric() || *d == '_') {
                j += 1;
            }
            let word = text.get(start..end_of(j)).unwrap_or("");
            if PY_KEYWORDS.contains(&word) {
                out.push((start..end_of(j), Token::Keyword));
            } else if PY_BUILTINS.contains(&word) {
                out.push((start..end_of(j), Token::Builtin));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

fn latex(text: &str) -> Vec<(Range<usize>, Token)> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let end_of = |i: usize| chars.get(i).map(|(b, _)| *b).unwrap_or(text.len());
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&(start, c)) = chars.get(i) {
        match c {
            '%' => {
                let mut j = i;
                while chars.get(j).is_some_and(|(_, c)| *c != '\n') {
                    j += 1;
                }
                out.push((start..end_of(j), Token::Comment));
                i = j;
            }
            '\\' => {
                let mut j = i + 1;
                if chars.get(j).is_some_and(|(_, d)| d.is_ascii_alphabetic()) {
                    while chars.get(j).is_some_and(|(_, d)| d.is_ascii_alphabetic()) {
                        j += 1;
                    }
                } else if chars.get(j).is_some() {
                    j += 1; // a control symbol such as \{ or \%
                }
                out.push((start..end_of(j), Token::Keyword));
                i = j;
            }
            '^' | '_' | '&' => {
                out.push((start..end_of(i + 1), Token::Builtin));
                i += 1;
            }
            d if d.is_ascii_digit() => {
                let mut j = i + 1;
                while chars.get(j).is_some_and(|(_, d)| d.is_ascii_digit() || *d == '.') {
                    j += 1;
                }
                out.push((start..end_of(j), Token::Number));
                i = j;
            }
            _ => i += 1,
        }
    }
    out
}
