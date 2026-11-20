//! Colours of the languages a Markdown code block names (```` ```java ````), beyond the code
//! editor's own Python, LaTeX and SQL: the C family (Java, Kotlin, JavaScript and TypeScript, C,
//! C++ and C#, Go, Rust and the like), JSON, XML and HTML (MyBatis's `#{…}` too), YAML, TOML,
//! INI and properties files, shell and PowerShell scripts, CSS and diffs. Each is a light
//! tokenizer, as editors colour as one types: it never fails, and what it doesn't know stays
//! plain.

use std::ops::Range;

use super::{Lang, Token};

/// The language a code block's info word names (`sql`, `Java`, `ts`, `yml`…); plain text for one
/// not known.
pub fn lang_of(name: &str) -> Lang {
    let name = name.trim().trim_start_matches('{').trim_start_matches('.').trim_end_matches('}').to_ascii_lowercase();
    match name.as_str() {
        "sql" | "plsql" | "pgsql" | "postgresql" | "postgres" | "psql" | "mysql" | "mariadb" | "tsql" | "mssql" | "oracle" | "sqlite" | "tibero" => {
            Lang::Sql
        }
        "py" | "python" | "python3" | "py3" => Lang::Python,
        "tex" | "latex" | "math" => Lang::Latex,
        "java" | "scala" | "groovy" | "gradle" => Lang::Java,
        "kotlin" | "kt" | "kts" => Lang::Kotlin,
        "js" | "javascript" | "jsx" | "mjs" | "cjs" | "ts" | "typescript" | "tsx" | "node" => Lang::Script,
        "c" | "h" | "cpp" | "c++" | "cc" | "cxx" | "hpp" | "objc" | "objective-c" => Lang::C,
        "cs" | "csharp" | "c#" => Lang::CSharp,
        "go" | "golang" => Lang::Go,
        "rust" | "rs" => Lang::Rust,
        "swift" | "dart" | "php" => Lang::Script,
        "json" | "jsonc" | "json5" | "geojson" => Lang::Json,
        "xml" | "html" | "htm" | "xhtml" | "svg" | "vue" | "jsp" | "mybatis" | "xsd" | "xsl" | "wsdl" | "plist" => Lang::Markup,
        "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf" | "properties" | "props" | "env" | "dotenv" | "editorconfig" => Lang::Config,
        "sh" | "bash" | "shell" | "zsh" | "console" | "terminal" | "ps1" | "powershell" | "pwsh" | "ps" | "bat" | "cmd" | "batch" | "dockerfile"
        | "docker" | "makefile" | "make" => Lang::Shell,
        "css" | "scss" | "sass" | "less" => Lang::Css,
        "diff" | "patch" => Lang::Diff,
        _ => Lang::Plain,
    }
}

const JAVA: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "float",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "null",
    "package",
    "permits",
    "private",
    "protected",
    "public",
    "record",
    "return",
    "sealed",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "true",
    "try",
    "var",
    "void",
    "volatile",
    "while",
    "yield",
    "def",
    "val",
    "object",
    "trait",
    "with",
    "match",
    "lazy",
    "implicit",
    "override",
];

const KOTLIN: &[&str] = &[
    "abstract",
    "annotation",
    "as",
    "break",
    "by",
    "catch",
    "class",
    "companion",
    "const",
    "constructor",
    "continue",
    "crossinline",
    "data",
    "do",
    "else",
    "enum",
    "external",
    "false",
    "final",
    "finally",
    "for",
    "fun",
    "get",
    "if",
    "import",
    "in",
    "infix",
    "init",
    "inline",
    "inner",
    "interface",
    "internal",
    "is",
    "lateinit",
    "noinline",
    "null",
    "object",
    "open",
    "operator",
    "out",
    "override",
    "package",
    "private",
    "protected",
    "public",
    "reified",
    "return",
    "sealed",
    "set",
    "super",
    "suspend",
    "this",
    "throw",
    "true",
    "try",
    "typealias",
    "val",
    "var",
    "vararg",
    "when",
    "where",
    "while",
];

const SCRIPT: &[&str] = &[
    "abstract",
    "any",
    "as",
    "async",
    "await",
    "boolean",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "constructor",
    "continue",
    "debugger",
    "declare",
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
    "func",
    "function",
    "get",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "is",
    "keyof",
    "let",
    "namespace",
    "never",
    "new",
    "null",
    "number",
    "of",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "set",
    "static",
    "string",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "unknown",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "echo",
    "elseif",
    "endif",
    "foreach",
    "guard",
    "let",
    "nil",
    "self",
    "struct",
    "protocol",
    "extension",
    "final",
    "late",
    "required",
    "dynamic",
];

const C: &[&str] = &[
    "auto",
    "bool",
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
    "explicit",
    "extern",
    "false",
    "float",
    "for",
    "friend",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "mutable",
    "namespace",
    "new",
    "noexcept",
    "nullptr",
    "NULL",
    "operator",
    "override",
    "private",
    "protected",
    "public",
    "register",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
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
    "virtual",
    "void",
    "volatile",
    "while",
];

const CSHARP: &[&str] = &[
    "abstract",
    "as",
    "async",
    "await",
    "base",
    "bool",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "decimal",
    "default",
    "delegate",
    "do",
    "double",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "false",
    "finally",
    "fixed",
    "float",
    "for",
    "foreach",
    "get",
    "goto",
    "if",
    "implicit",
    "in",
    "init",
    "int",
    "interface",
    "internal",
    "is",
    "lock",
    "long",
    "namespace",
    "new",
    "null",
    "object",
    "operator",
    "out",
    "override",
    "params",
    "partial",
    "private",
    "protected",
    "public",
    "readonly",
    "record",
    "ref",
    "return",
    "sealed",
    "set",
    "short",
    "sizeof",
    "static",
    "string",
    "struct",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "uint",
    "ulong",
    "using",
    "var",
    "virtual",
    "void",
    "volatile",
    "where",
    "while",
    "yield",
];

const GO: &[&str] = &[
    "any",
    "bool",
    "break",
    "byte",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "error",
    "fallthrough",
    "false",
    "float32",
    "float64",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "int",
    "int32",
    "int64",
    "interface",
    "iota",
    "map",
    "nil",
    "package",
    "range",
    "return",
    "rune",
    "select",
    "string",
    "struct",
    "switch",
    "true",
    "type",
    "uint",
    "var",
];

const RUST: &[&str] = &[
    "as", "async", "await", "bool", "break", "char", "const", "continue", "crate", "dyn", "else", "enum", "extern", "f32", "f64", "false", "fn",
    "for", "i8", "i16", "i32", "i64", "i128", "if", "impl", "in", "isize", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "str", "struct", "super", "trait", "true", "type", "u8", "u16", "u32", "u64", "u128", "unsafe", "use", "usize",
    "where", "while",
];

const SHELL: &[&str] = &[
    "begin",
    "break",
    "call",
    "case",
    "cd",
    "continue",
    "do",
    "done",
    "echo",
    "elif",
    "else",
    "end",
    "esac",
    "exit",
    "export",
    "fi",
    "for",
    "foreach",
    "from",
    "function",
    "goto",
    "if",
    "in",
    "local",
    "param",
    "process",
    "read",
    "return",
    "run",
    "set",
    "shift",
    "source",
    "switch",
    "then",
    "throw",
    "trap",
    "try",
    "catch",
    "finally",
    "unset",
    "until",
    "while",
    "workdir",
    "copy",
    "add",
    "cmd",
    "entrypoint",
    "env",
    "expose",
    "arg",
    "volume",
    "user",
    "label",
    "rem",
    "setlocal",
    "endlocal",
];

/// How a C-family language is read: its keywords, what `#` starts, whether `` ` `` quotes a
/// string, `@` names an annotation and `'a` is a Rust lifetime.
struct Family {
    keywords: &'static [&'static str],
    hash: Hash,
    backtick: bool,
    lifetimes: bool,
}

#[derive(PartialEq)]
enum Hash {
    /// `#` is an operator (Java, JavaScript…).
    Plain,
    /// `#include`, `#region`: a directive, to the end of its line.
    Directive,
    /// `#[derive]`: an attribute (Rust).
    Attribute,
}

/// The coloured spans of `text` in `lang` (one of this module's).
pub fn highlight(text: &str, lang: Lang) -> Vec<(Range<usize>, Token)> {
    let family = |keywords, hash, backtick, lifetimes| Family { keywords, hash, backtick, lifetimes };
    match lang {
        Lang::Java => c_family(text, &family(JAVA, Hash::Plain, false, false)),
        Lang::Kotlin => c_family(text, &family(KOTLIN, Hash::Plain, false, false)),
        Lang::Script => c_family(text, &family(SCRIPT, Hash::Plain, true, false)),
        Lang::C => c_family(text, &family(C, Hash::Directive, false, false)),
        Lang::CSharp => c_family(text, &family(CSHARP, Hash::Directive, false, false)),
        Lang::Go => c_family(text, &family(GO, Hash::Plain, true, false)),
        Lang::Rust => c_family(text, &family(RUST, Hash::Attribute, false, true)),
        Lang::Json => json(text),
        Lang::Markup => markup(text),
        Lang::Config => config(text),
        Lang::Shell => shell(text),
        Lang::Css => css(text),
        Lang::Diff => diff(text),
        Lang::Python | Lang::Latex | Lang::Sql | Lang::Plain => Vec::new(),
    }
}

/// A text read char by char: its chars with their byte offsets.
struct Chars<'a> {
    text: &'a str,
    chars: Vec<(usize, char)>,
}

impl<'a> Chars<'a> {
    fn new(text: &'a str) -> Self {
        Chars { text, chars: text.char_indices().collect() }
    }
    fn at(&self, i: usize) -> Option<char> {
        self.chars.get(i).map(|(_, c)| *c)
    }
    /// The byte offset of char `i` (the text's end past the last).
    fn byte(&self, i: usize) -> usize {
        self.chars.get(i).map_or(self.text.len(), |(b, _)| *b)
    }
    fn starts(&self, i: usize, s: &str) -> bool {
        self.text.get(self.byte(i)..).is_some_and(|rest| rest.starts_with(s))
    }
    /// The first char at or after `i` that `stop` picks (the end when none does).
    fn until(&self, mut i: usize, stop: impl Fn(char) -> bool) -> usize {
        while self.at(i).is_some_and(|c| !stop(c)) {
            i += 1;
        }
        i
    }
    /// The char after the first `close` at or after `i` (the end when there is none).
    fn past(&self, i: usize, close: &str) -> usize {
        let from = self.byte(i);
        match self.text.get(from..).and_then(|rest| rest.find(close)) {
            Some(at) => {
                let end = from + at + close.len();
                self.chars.partition_point(|(b, _)| *b < end)
            }
            None => self.chars.len(),
        }
    }
    /// The char after a string opening with `quote` at `i`: past its closing quote, `\` escaping
    /// the next char; one that isn't `multiline` ends at its line's end.
    fn string_end(&self, i: usize, quote: char, multiline: bool) -> usize {
        let mut j = i + 1;
        loop {
            match self.at(j) {
                None => return j,
                Some('\\') => j += 2,
                Some('\n') if !multiline => return j,
                Some(c) if c == quote => return j + 1,
                Some(_) => j += 1,
            }
        }
    }
    fn number_end(&self, i: usize) -> usize {
        self.until(i + 1, |c| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
    }
    fn word_end(&self, i: usize) -> usize {
        self.until(i, |c| !(c.is_alphanumeric() || c == '_' || c == '$'))
    }
    fn span(&self, from: usize, to: usize, token: Token, out: &mut Vec<(Range<usize>, Token)>) {
        let (a, b) = (self.byte(from), self.byte(to));
        if a < b {
            out.push((a..b, token));
        }
    }
    /// The next char that isn't white space at or after `i`.
    fn next_solid(&self, i: usize) -> Option<char> {
        self.at(self.until(i, |c| !c.is_whitespace()))
    }
}

fn c_family(text: &str, f: &Family) -> Vec<(Range<usize>, Token)> {
    let s = Chars::new(text);
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(c) = s.at(i) {
        let j = if s.starts(i, "//") {
            let j = s.until(i, |c| c == '\n');
            s.span(i, j, Token::Comment, &mut out);
            j
        } else if s.starts(i, "/*") {
            let j = s.past(i + 2, "*/");
            s.span(i, j, Token::Comment, &mut out);
            j
        } else if c == '"' || (c == '`' && f.backtick) {
            let j = if s.starts(i, "\"\"\"") { s.past(i + 3, "\"\"\"") } else { s.string_end(i, c, c == '`') };
            s.span(i, j, Token::String, &mut out);
            j
        } else if c == '\'' && f.lifetimes && s.at(i + 1).is_some_and(|d| d.is_alphabetic() || d == '_') && s.at(i + 2) != Some('\'') {
            // A Rust lifetime (`'a`), not a char.
            let j = s.word_end(i + 1);
            s.span(i, j, Token::Builtin, &mut out);
            j
        } else if c == '\'' {
            let j = s.string_end(i, c, false);
            s.span(i, j, Token::String, &mut out);
            j
        } else if c == '#' && f.hash == Hash::Directive && s.at(i + 1).is_some_and(|d| d.is_ascii_alphabetic()) {
            let j = s.word_end(i + 1);
            s.span(i, j, Token::Keyword, &mut out);
            j
        } else if c == '#' && f.hash == Hash::Attribute && matches!(s.at(i + 1), Some('[' | '!')) {
            let j = s.past(i, "]");
            s.span(i, j, Token::Builtin, &mut out);
            j
        } else if c == '@' && s.at(i + 1).is_some_and(|d| d.is_alphabetic()) {
            // An annotation or decorator (`@Override`, `@Component`).
            let j = s.word_end(i + 1);
            s.span(i, j, Token::Builtin, &mut out);
            j
        } else if c.is_ascii_digit() {
            let j = s.number_end(i);
            s.span(i, j, Token::Number, &mut out);
            j
        } else if c.is_alphabetic() || c == '_' || c == '$' {
            let j = s.word_end(i);
            let word = text.get(s.byte(i)..s.byte(j)).unwrap_or("");
            if f.keywords.contains(&word) {
                s.span(i, j, Token::Keyword, &mut out);
            } else if s.next_solid(j) == Some('(') || (f.lifetimes && s.at(j) == Some('!')) {
                // A function called (or a Rust macro).
                s.span(i, j, Token::Builtin, &mut out);
            }
            j
        } else {
            i + 1
        };
        i = j.max(i + 1);
    }
    out
}

fn json(text: &str) -> Vec<(Range<usize>, Token)> {
    let s = Chars::new(text);
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(c) = s.at(i) {
        let j = if s.starts(i, "//") {
            let j = s.until(i, |c| c == '\n');
            s.span(i, j, Token::Comment, &mut out);
            j
        } else if s.starts(i, "/*") {
            let j = s.past(i + 2, "*/");
            s.span(i, j, Token::Comment, &mut out);
            j
        } else if c == '"' || c == '\'' {
            let j = s.string_end(i, c, false);
            // A key is a string before a colon.
            let token = if s.next_solid(j) == Some(':') { Token::Builtin } else { Token::String };
            s.span(i, j, token, &mut out);
            j
        } else if c.is_ascii_digit() || (c == '-' && s.at(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let j = s.until(i + 1, |c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-')));
            s.span(i, j, Token::Number, &mut out);
            j
        } else if c.is_ascii_alphabetic() {
            let j = s.word_end(i);
            if matches!(text.get(s.byte(i)..s.byte(j)), Some("true" | "false" | "null")) {
                s.span(i, j, Token::Keyword, &mut out);
            }
            j
        } else {
            i + 1
        };
        i = j.max(i + 1);
    }
    out
}

fn markup(text: &str) -> Vec<(Range<usize>, Token)> {
    let s = Chars::new(text);
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(c) = s.at(i) {
        let j = if s.starts(i, "<!--") {
            let j = s.past(i + 4, "-->");
            s.span(i, j, Token::Comment, &mut out);
            j
        } else if s.starts(i, "<%--") {
            // A JSP comment.
            let j = s.past(i + 4, "--%>");
            s.span(i, j, Token::Comment, &mut out);
            j
        } else if s.starts(i, "<%") {
            // A JSP scriptlet, expression or directive: Java between `<%` and `%>`.
            let open = s.until(i + 2, |c| !matches!(c, '=' | '!' | '@'));
            s.span(i, open, Token::Keyword, &mut out);
            let end = s.past(open, "%>");
            let close = if s.starts(end.saturating_sub(2), "%>") { end - 2 } else { end };
            embed(&s, (open, close), Lang::Java, &mut out);
            s.span(close, end, Token::Keyword, &mut out);
            end
        } else if s.starts(i, "<![CDATA[") {
            let j = s.past(i + 9, "]]>");
            s.span(i, j, Token::String, &mut out);
            j
        } else if c == '<' && s.at(i + 1).is_some_and(|d| d.is_alphabetic() || matches!(d, '/' | '?' | '!')) {
            let j = tag(&s, i, &mut out);
            // The script and style of a page (HTML, Vue, JSP): JavaScript and CSS up to its end tag.
            let name: String = (i + 1..).map_while(|k| s.at(k).filter(|c| c.is_ascii_alphanumeric())).collect::<String>().to_ascii_lowercase();
            let closed = j >= 2 && s.at(j - 2) == Some('/');
            match (name.as_str(), closed) {
                ("script", false) | ("style", false) => {
                    let lang = if name == "script" { Lang::Script } else { Lang::Css };
                    let from = s.byte(j);
                    let rest = s.text.get(from..).unwrap_or_default().to_ascii_lowercase();
                    let end_byte = rest.find(&format!("</{name}")).map_or(s.text.len(), |at| from + at);
                    let end = s.chars.partition_point(|(b, _)| *b < end_byte);
                    embed(&s, (j, end), lang, &mut out);
                    end
                }
                _ => j,
            }
        } else if (c == '#' || c == '$') && s.at(i + 1) == Some('{') {
            // MyBatis's parameters (`#{id}`, `${table}`) in a mapper's SQL.
            let j = s.past(i + 2, "}");
            s.span(i, j, Token::Builtin, &mut out);
            j
        } else if c == '&' {
            let j = s.until(i + 1, |c| !(c.is_alphanumeric() || c == '#'));
            if s.at(j) == Some(';') {
                s.span(i, j + 1, Token::Number, &mut out);
            }
            j
        } else {
            i + 1
        };
        i = j.max(i + 1);
    }
    out
}

/// Colours the chars `from..to` of `s` as `lang` (a page's script or style, a JSP's Java).
fn embed(s: &Chars, (from, to): (usize, usize), lang: Lang, out: &mut Vec<(Range<usize>, Token)>) {
    let (a, b) = (s.byte(from), s.byte(to));
    let Some(inner) = s.text.get(a..b) else { return };
    out.extend(highlight(inner, lang).into_iter().map(|(r, t)| (r.start + a..r.end + a, t)));
}

/// A tag at `i` (its `<`): its name a keyword, its attributes' names built-ins and their values
/// strings. The char after it.
fn tag(s: &Chars, i: usize, out: &mut Vec<(Range<usize>, Token)>) -> usize {
    let start = s.until(i + 1, |c| !matches!(c, '/' | '?' | '!'));
    let name_end = s.until(start, |c| c.is_whitespace() || matches!(c, '>' | '/' | '?'));
    s.span(i, name_end, Token::Keyword, out);
    let mut j = name_end;
    while let Some(c) = s.at(j) {
        match c {
            '>' => return j + 1,
            '"' | '\'' => {
                let k = s.string_end(j, c, true);
                s.span(j, k, Token::String, out);
                j = k;
            }
            c if c.is_alphabetic() || c == '_' || c == ':' => {
                let k = s.until(j, |c| c.is_whitespace() || matches!(c, '=' | '>' | '/'));
                s.span(j, k, Token::Builtin, out);
                j = k.max(j + 1);
            }
            _ => j += 1,
        }
    }
    j
}

fn config(text: &str) -> Vec<(Range<usize>, Token)> {
    let s = Chars::new(text);
    let mut out = Vec::new();
    let mut i = 0;
    while s.at(i).is_some() {
        let end = s.until(i, |c| c == '\n');
        let first = s.until(i, |c| !(c == ' ' || c == '\t'));
        match s.at(first) {
            Some('#' | ';') => s.span(first, end, Token::Comment, &mut out),
            Some('[') => s.span(first, end, Token::Keyword, &mut out),
            Some(_) => {
                let lead = if s.starts(first, "- ") { first + 2 } else { first };
                // `key: value`, `key = value`, `key=value`.
                let sep = s.until(lead, |c| c == ':' || c == '=' || c == '\n');
                let value_from = if s.at(sep).is_some_and(|c| c == ':' || c == '=') && sep < end {
                    s.span(lead, sep, Token::Builtin, &mut out);
                    sep + 1
                } else {
                    lead
                };
                value(&s, value_from, end, &mut out);
            }
            None => {}
        }
        i = end + 1;
    }
    out
}

/// A setting's value from `i` to the line's end `end`: a quoted string, a number or a word such as
/// `true`, then a ` #` comment.
fn value(s: &Chars, i: usize, end: usize, out: &mut Vec<(Range<usize>, Token)>) {
    let from = s.until(i, |c| !(c == ' ' || c == '\t'));
    let comment = (from..end).find(|&k| s.at(k) == Some('#') && (k == from || s.at(k.saturating_sub(1)).is_some_and(char::is_whitespace)));
    let body_end = comment.unwrap_or(end);
    match s.at(from) {
        Some(q @ ('"' | '\'')) => {
            let k = s.string_end(from, q, false).min(body_end);
            s.span(from, k, Token::String, out);
        }
        Some(c) if c.is_ascii_digit() || c == '-' => {
            let word = s.text.get(s.byte(from)..s.byte(body_end)).unwrap_or("").trim();
            if word.parse::<f64>().is_ok() {
                s.span(from, body_end, Token::Number, out);
            }
        }
        Some(_) => {
            let word = s.text.get(s.byte(from)..s.byte(body_end)).unwrap_or("").trim().to_ascii_lowercase();
            if matches!(word.as_str(), "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~") {
                s.span(from, body_end, Token::Keyword, out);
            }
        }
        None => {}
    }
    if let Some(k) = comment {
        s.span(k, end, Token::Comment, out);
    }
}

fn shell(text: &str) -> Vec<(Range<usize>, Token)> {
    let s = Chars::new(text);
    let mut out = Vec::new();
    let mut i = 0;
    let mut line_start = true;
    while let Some(c) = s.at(i) {
        let j = if c == '#' && (i == 0 || s.at(i - 1).is_some_and(char::is_whitespace)) {
            let j = s.until(i, |c| c == '\n');
            s.span(i, j, Token::Comment, &mut out);
            j
        } else if line_start && (s.starts(i, "::") || s.starts(i, "REM ") || s.starts(i, "rem ")) {
            let j = s.until(i, |c| c == '\n');
            s.span(i, j, Token::Comment, &mut out);
            j
        } else if c == '"' || c == '\'' {
            let j = s.string_end(i, c, true);
            s.span(i, j, Token::String, &mut out);
            j
        } else if c == '$' || c == '%' {
            // `$name`, `${name}`, `$env:Path`, `%PATH%`.
            let j = match s.at(i + 1) {
                Some('{') => s.past(i + 2, "}"),
                Some('(') => i + 1,
                Some(d) if c == '%' && d.is_alphabetic() => s.past(i + 1, "%"),
                Some(d) if c == '$' && (d.is_alphanumeric() || matches!(d, '_' | '?' | '@' | '#')) => {
                    s.until(i + 1, |c| !(c.is_alphanumeric() || matches!(c, '_' | ':')))
                }
                _ => i + 1,
            };
            if j > i + 1 {
                s.span(i, j, Token::Builtin, &mut out);
            }
            j
        } else if c.is_ascii_digit() && (i == 0 || s.at(i - 1).is_some_and(|p| !(p.is_alphanumeric() || p == '_' || p == '-'))) {
            let j = s.number_end(i);
            s.span(i, j, Token::Number, &mut out);
            j
        } else if c.is_alphabetic() || c == '_' {
            let j = s.until(i, |c| !(c.is_alphanumeric() || c == '_' || c == '-'));
            let word = s.text.get(s.byte(i)..s.byte(j)).unwrap_or("").to_ascii_lowercase();
            if SHELL.contains(&word.as_str()) {
                s.span(i, j, Token::Keyword, &mut out);
            }
            j
        } else {
            i + 1
        };
        line_start = s.at(j.saturating_sub(1)) == Some('\n') || (line_start && c.is_whitespace() && c != '\n');
        if s.at(j) == Some('\n') {
            line_start = true;
        }
        i = j.max(i + 1);
    }
    out
}

fn css(text: &str) -> Vec<(Range<usize>, Token)> {
    let s = Chars::new(text);
    let mut out = Vec::new();
    let mut i = 0;
    let mut depth = 0usize;
    while let Some(c) = s.at(i) {
        let j = if s.starts(i, "/*") {
            let j = s.past(i + 2, "*/");
            s.span(i, j, Token::Comment, &mut out);
            j
        } else if s.starts(i, "//") {
            let j = s.until(i, |c| c == '\n');
            s.span(i, j, Token::Comment, &mut out);
            j
        } else if c == '"' || c == '\'' {
            let j = s.string_end(i, c, false);
            s.span(i, j, Token::String, &mut out);
            j
        } else if c == '{' || c == '}' {
            depth = if c == '{' { depth + 1 } else { depth.saturating_sub(1) };
            i + 1
        } else if c == '@' || c == '!' {
            let j = s.word_end(i + 1);
            s.span(i, j, Token::Keyword, &mut out);
            j
        } else if c == '#' && depth > 0 {
            let j = s.word_end(i + 1);
            s.span(i, j, Token::Number, &mut out);
            j
        } else if c.is_ascii_digit() || (c == '.' && s.at(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let j = s.until(i + 1, |c| !(c.is_ascii_alphanumeric() || c == '.' || c == '%'));
            s.span(i, j, Token::Number, &mut out);
            j
        } else if depth > 0 && (c.is_alphabetic() || c == '-') {
            let j = s.until(i, |c| !(c.is_alphanumeric() || c == '-' || c == '_'));
            if s.next_solid(j) == Some(':') {
                s.span(i, j, Token::Builtin, &mut out);
            }
            j
        } else if depth == 0 && (c.is_alphabetic() || matches!(c, '.' | '#' | ':')) {
            // A selector.
            let j = s.until(i + 1, |c| !(c.is_alphanumeric() || matches!(c, '-' | '_')));
            s.span(i, j, Token::Keyword, &mut out);
            j
        } else {
            i + 1
        };
        i = j.max(i + 1);
    }
    out
}

fn diff(text: &str) -> Vec<(Range<usize>, Token)> {
    let mut out = Vec::new();
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let token = if body.starts_with("+++") || body.starts_with("---") || body.starts_with("diff ") || body.starts_with("index ") {
            Some(Token::Comment)
        } else if body.starts_with("@@") {
            Some(Token::Keyword)
        } else if body.starts_with('+') {
            Some(Token::Number)
        } else if body.starts_with('-') {
            Some(Token::String)
        } else {
            None
        };
        if let Some(token) = token.filter(|_| !body.is_empty()) {
            out.push((at..at + body.len(), token));
        }
        at += line.len();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text of each coloured span with its kind.
    fn spans(text: &str, lang: Lang) -> Vec<(&str, Token)> {
        highlight(text, lang).into_iter().map(|(r, t)| (text.get(r).unwrap(), t)).collect()
    }

    fn has(found: &[(&str, Token)], text: &str, token: Token) -> bool {
        found.iter().any(|(s, t)| *s == text && *t == token)
    }

    #[test]
    fn code_block_names_pick_their_language() {
        assert_eq!(lang_of("sql"), Lang::Sql);
        assert_eq!(lang_of("PLSQL"), Lang::Sql);
        assert_eq!(lang_of("Java"), Lang::Java);
        assert_eq!(lang_of("ts"), Lang::Script);
        assert_eq!(lang_of("{.yaml}"), Lang::Config);
        assert_eq!(lang_of("xml"), Lang::Markup);
        assert_eq!(lang_of("ps1"), Lang::Shell);
        assert_eq!(lang_of("klingon"), Lang::Plain);
        assert_eq!(lang_of(""), Lang::Plain);
    }

    #[test]
    fn java_keywords_strings_comments_and_calls() {
        let found = spans("@Override\npublic String name(int n) { // 이름\n  return \"값\" + list.size() + 42; /* 끝 */ }", Lang::Java);
        assert!(has(&found, "@Override", Token::Builtin));
        assert!(has(&found, "public", Token::Keyword) && has(&found, "return", Token::Keyword) && has(&found, "int", Token::Keyword));
        assert!(has(&found, "\"값\"", Token::String));
        assert!(has(&found, "// 이름", Token::Comment) && has(&found, "/* 끝 */", Token::Comment));
        assert!(has(&found, "size", Token::Builtin) && has(&found, "name", Token::Builtin));
        assert!(has(&found, "42", Token::Number));
        assert!(!found.iter().any(|(s, _)| *s == "list" || *s == "String"));
    }

    #[test]
    fn script_template_strings_and_rust_lifetimes() {
        let found = spans("const s = `a ${b}\nc`; let x = 'q';", Lang::Script);
        assert!(has(&found, "`a ${b}\nc`", Token::String) && has(&found, "'q'", Token::String) && has(&found, "const", Token::Keyword));
        let found = spans("fn f<'a>(s: &'a str) -> char { println!(\"{}\", 'x'); }", Lang::Rust);
        assert!(has(&found, "'a", Token::Builtin) && has(&found, "'x'", Token::String) && has(&found, "println", Token::Builtin));
        assert!(has(&found, "fn", Token::Keyword) && has(&found, "str", Token::Keyword));
        let found = spans("#include <stdio.h>\nint main() {}", Lang::C);
        assert!(has(&found, "#include", Token::Keyword) && has(&found, "int", Token::Keyword));
    }

    #[test]
    fn json_keys_values_and_words() {
        let found = spans("{\"이름\": \"값\", \"n\": -1.5e3, \"ok\": true, \"none\": null}", Lang::Json);
        assert!(has(&found, "\"이름\"", Token::Builtin) && has(&found, "\"값\"", Token::String));
        assert!(has(&found, "-1.5e3", Token::Number) && has(&found, "true", Token::Keyword) && has(&found, "null", Token::Keyword));
    }

    #[test]
    fn markup_tags_attributes_and_mybatis_parameters() {
        let found = spans("<?xml version=\"1.0\"?>\n<!-- 메모 -->\n<select id=\"find\">SELECT * FROM T WHERE ID = #{id}</select>", Lang::Markup);
        assert!(has(&found, "<select", Token::Keyword) && has(&found, "</select", Token::Keyword) && has(&found, "<?xml", Token::Keyword));
        assert!(has(&found, "id", Token::Builtin) && has(&found, "\"find\"", Token::String));
        assert!(has(&found, "<!-- 메모 -->", Token::Comment) && has(&found, "#{id}", Token::Builtin));
    }

    #[test]
    fn a_pages_script_style_and_jsp_java_take_their_own_colours() {
        let vue = "<template><p>{{ a }}</p></template>
<script setup>
const n = 1 // 메모
</script>
<style>.a { color: red; }</style>";
        let found = spans(vue, Lang::Markup);
        assert!(has(&found, "const", Token::Keyword) && has(&found, "1", Token::Number) && has(&found, "// 메모", Token::Comment));
        assert!(has(&found, "</script", Token::Keyword) && has(&found, "<style", Token::Keyword) && has(&found, "</style", Token::Keyword));
        let jsp = "<%@ page contentType=\"text/html\" %>
<%-- 목록 --%>
<% for (int i = 0; i < 3; i++) { %><td><%= i %></td><% } %>";
        let found = spans(jsp, Lang::Markup);
        assert!(has(&found, "<%--  목록 --%>".replace("  ", " ").as_str(), Token::Comment));
        assert!(has(&found, "for", Token::Keyword) && has(&found, "int", Token::Keyword) && has(&found, "\"text/html\"", Token::String));
        assert!(has(&found, "<%=", Token::Keyword) && has(&found, "<td", Token::Keyword));
        // An unclosed script or scriptlet colours to the end and never fails.
        for broken in ["<script>let a = '", "<% if (x", "<style", "<%", "<script/>x"] {
            let _ = spans(broken, Lang::Markup);
        }
    }

    #[test]
    fn config_keys_values_and_comments() {
        let found = spans("# 설정\n[server]\nport: 8080\nname = \"모눈\"\ndebug=true # 켜기\n- item\n", Lang::Config);
        assert!(has(&found, "# 설정", Token::Comment) && has(&found, "[server]", Token::Keyword));
        assert!(has(&found, "port", Token::Builtin) && has(&found, "8080", Token::Number));
        assert!(has(&found, "\"모눈\"", Token::String) && has(&found, "true ", Token::Keyword) && has(&found, "# 켜기", Token::Comment));
    }

    #[test]
    fn shell_comments_variables_and_keywords() {
        let found = spans("# 빌드\nif [ -f \"$HOME/a\" ]; then echo ${NAME} $env:Path; fi\nREM 배치", Lang::Shell);
        assert!(has(&found, "# 빌드", Token::Comment) && has(&found, "if", Token::Keyword) && has(&found, "fi", Token::Keyword));
        assert!(has(&found, "\"$HOME/a\"", Token::String) && has(&found, "${NAME}", Token::Builtin) && has(&found, "$env:Path", Token::Builtin));
        assert!(has(&found, "REM 배치", Token::Comment));
    }

    #[test]
    fn css_and_diff() {
        let found = spans(".box { color: #fff; margin: 4px !important; } /* 끝 */", Lang::Css);
        assert!(has(&found, ".box", Token::Keyword) && has(&found, "color", Token::Builtin) && has(&found, "#fff", Token::Number));
        assert!(has(&found, "4px", Token::Number) && has(&found, "!important", Token::Keyword) && has(&found, "/* 끝 */", Token::Comment));
        let found = spans("--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n same\n", Lang::Diff);
        assert!(has(&found, "-old", Token::String) && has(&found, "+new", Token::Number) && has(&found, "@@ -1 +1 @@", Token::Keyword));
        assert!(has(&found, "--- a", Token::Comment));
    }

    #[test]
    fn odd_input_never_panics_and_spans_stay_in_order() {
        let odd = ["", "\"", "'", "`", "/*", "<", "<!--", "<a b=\"", "#{", "${", "%", "@", "#", "'a", "\\", "[", "- ", ":", "=", "é\"ü", "\u{0}"];
        for lang in [
            Lang::Java,
            Lang::Kotlin,
            Lang::Script,
            Lang::C,
            Lang::CSharp,
            Lang::Go,
            Lang::Rust,
            Lang::Json,
            Lang::Markup,
            Lang::Config,
            Lang::Shell,
            Lang::Css,
            Lang::Diff,
        ] {
            for text in odd {
                let spans = highlight(text, lang);
                let mut end = 0;
                for (r, _) in &spans {
                    assert!(r.start >= end && r.end <= text.len() && text.get(r.clone()).is_some(), "{lang:?} {text:?} {spans:?}");
                    end = r.end;
                }
            }
        }
    }
}
