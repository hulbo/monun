//! Source files by name: the files of everyday development (Java and JSP, Vue and JavaScript,
//! YAML, XML, properties, shell scripts…) that open as text to read and edit, coloured as their
//! language and named as code editors' status bars name it ([`source_of`]).

use super::Lang;

/// A source file's language: what code editors' status bars call it, and its colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    pub label: &'static str,
    pub lang: Lang,
}

/// The source files by extension (lower case). SQL, Markdown, `.http` requests, plain `.txt` and
/// web pages (`.html`) are files of their own kinds and aren't here.
const BY_EXTENSION: &[(&str, &str, Lang)] = &[
    ("java", "Java", Lang::Java),
    ("kt", "Kotlin", Lang::Kotlin),
    ("kts", "Kotlin", Lang::Kotlin),
    ("groovy", "Groovy", Lang::Java),
    ("gradle", "Gradle", Lang::Java),
    ("scala", "Scala", Lang::Java),
    ("jsp", "JSP", Lang::Markup),
    ("jspf", "JSP", Lang::Markup),
    ("tag", "JSP", Lang::Markup),
    ("tld", "XML", Lang::Markup),
    ("js", "JavaScript", Lang::Script),
    ("mjs", "JavaScript", Lang::Script),
    ("cjs", "JavaScript", Lang::Script),
    ("jsx", "JavaScript JSX", Lang::Script),
    ("ts", "TypeScript", Lang::Script),
    ("mts", "TypeScript", Lang::Script),
    ("cts", "TypeScript", Lang::Script),
    ("tsx", "TypeScript JSX", Lang::Script),
    ("vue", "Vue", Lang::Markup),
    ("svelte", "Svelte", Lang::Markup),
    ("xml", "XML", Lang::Markup),
    ("xsd", "XML", Lang::Markup),
    ("xsl", "XSLT", Lang::Markup),
    ("xslt", "XSLT", Lang::Markup),
    ("wsdl", "XML", Lang::Markup),
    ("xhtml", "XHTML", Lang::Markup),
    ("svg", "SVG", Lang::Markup),
    ("plist", "XML", Lang::Markup),
    ("csproj", "XML", Lang::Markup),
    ("vbproj", "XML", Lang::Markup),
    ("ftl", "FreeMarker", Lang::Markup),
    ("vm", "Velocity", Lang::Markup),
    ("hbs", "Handlebars", Lang::Markup),
    ("mustache", "Mustache", Lang::Markup),
    ("ejs", "EJS", Lang::Markup),
    ("asp", "ASP", Lang::Markup),
    ("aspx", "ASP.NET", Lang::Markup),
    ("cshtml", "Razor", Lang::Markup),
    ("razor", "Razor", Lang::Markup),
    ("json", "JSON", Lang::Json),
    ("jsonc", "JSON", Lang::Json),
    ("json5", "JSON5", Lang::Json),
    ("yml", "YAML", Lang::Config),
    ("yaml", "YAML", Lang::Config),
    ("toml", "TOML", Lang::Config),
    ("ini", "INI", Lang::Config),
    ("cfg", "INI", Lang::Config),
    ("conf", "Config", Lang::Config),
    ("properties", "Properties", Lang::Config),
    ("env", "Environment", Lang::Config),
    ("tf", "Terraform", Lang::Config),
    ("css", "CSS", Lang::Css),
    ("scss", "SCSS", Lang::Css),
    ("sass", "Sass", Lang::Css),
    ("less", "Less", Lang::Css),
    ("sh", "Shell", Lang::Shell),
    ("bash", "Shell", Lang::Shell),
    ("zsh", "Shell", Lang::Shell),
    ("bat", "Batch", Lang::Shell),
    ("cmd", "Batch", Lang::Shell),
    ("ps1", "PowerShell", Lang::Shell),
    ("psm1", "PowerShell", Lang::Shell),
    ("psd1", "PowerShell", Lang::Shell),
    ("py", "Python", Lang::Python),
    ("c", "C", Lang::C),
    ("h", "C", Lang::C),
    ("cpp", "C++", Lang::C),
    ("cc", "C++", Lang::C),
    ("cxx", "C++", Lang::C),
    ("hpp", "C++", Lang::C),
    ("hh", "C++", Lang::C),
    ("cs", "C#", Lang::CSharp),
    ("go", "Go", Lang::Go),
    ("rs", "Rust", Lang::Rust),
    ("swift", "Swift", Lang::Script),
    ("dart", "Dart", Lang::Script),
    ("php", "PHP", Lang::Script),
    ("tex", "LaTeX", Lang::Latex),
    ("diff", "Diff", Lang::Diff),
    ("patch", "Diff", Lang::Diff),
    ("log", "Log", Lang::Plain),
    ("rb", "Ruby", Lang::Plain),
    ("pl", "Perl", Lang::Plain),
    ("pm", "Perl", Lang::Plain),
    ("lua", "Lua", Lang::Plain),
    ("r", "R", Lang::Plain),
    ("vb", "Visual Basic", Lang::Plain),
    ("vbs", "VBScript", Lang::Plain),
    ("proto", "Protocol Buffers", Lang::Plain),
    ("graphql", "GraphQL", Lang::Plain),
    ("gql", "GraphQL", Lang::Plain),
];

/// The source files known by their whole name (lower case), having no extension of their own.
const BY_NAME: &[(&str, &str, Lang)] = &[
    ("dockerfile", "Dockerfile", Lang::Shell),
    ("makefile", "Makefile", Lang::Shell),
    ("jenkinsfile", "Jenkinsfile", Lang::Java),
    ("vagrantfile", "Ruby", Lang::Plain),
    (".gitignore", "Ignore", Lang::Shell),
    (".gitattributes", "Git", Lang::Shell),
    (".dockerignore", "Ignore", Lang::Shell),
    (".npmrc", "INI", Lang::Config),
    (".nvmrc", "Config", Lang::Config),
    (".editorconfig", "EditorConfig", Lang::Config),
    (".env", "Environment", Lang::Config),
    (".babelrc", "JSON", Lang::Json),
    (".eslintrc", "JSON", Lang::Json),
    (".prettierrc", "JSON", Lang::Json),
];

/// The source file called `name` (a file name or a path): its language, or `None` for a file
/// that isn't one (a workbook, a Markdown or SQL file, a picture…).
pub fn source_of(name: &str) -> Option<Source> {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    let found = |table: &[(&str, &'static str, Lang)], key: &str| {
        table.iter().find(|(k, _, _)| *k == key).map(|(_, label, lang)| Source { label, lang: *lang })
    };
    if let Some(s) = found(BY_NAME, &file) {
        return Some(s);
    }
    // `.env.local`, `.env.production`: what `.env` is.
    if file.starts_with(".env.") {
        return found(BY_NAME, ".env");
    }
    let (stem, ext) = file.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    found(BY_EXTENSION, ext)
}

/// Every extension [`source_of`] knows, for a file dialog's filter.
pub fn source_extensions() -> impl Iterator<Item = &'static str> {
    BY_EXTENSION.iter().map(|(ext, _, _)| *ext)
}
