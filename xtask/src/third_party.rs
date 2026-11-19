//! `cargo xtask third-party <out.txt>`: the licences of the crates built into monun.exe, in one
//! text file the installer puts beside the app (`packaging/monun.iss`, `scripts/release.ps1`).
//!
//! The crates are those the desktop app reaches through normal dependencies on Windows (build
//! scripts' and tests' crates and proc-macros don't end up in the exe). Each is listed with its
//! licence expression and source; the licence and notice files they ship follow, each distinct
//! text once, with the crates it covers. A crate from a git checkout whose folder has none takes
//! those of its repository's folders above it.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};

/// The package whose exe the setup ships.
const APP: &str = "monun";
/// The platform the setup is for.
const TARGET: &str = "x86_64-pc-windows-msvc";
/// File names (upper-cased prefixes) of the licence and notice files a crate ships.
const PREFIXES: &[&str] = &["LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "NOTICE", "UNLICENSE"];

/// One crate built into the exe.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Crate {
    pub name: String,
    pub version: String,
    pub license: String,
    pub source: String,
    /// Its licence and notice texts, in file-name order.
    pub texts: Vec<String>,
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let [out] = args else { return Err("usage: cargo xtask third-party <out.txt>".into()) };
    let output = crate::cargo()
        .current_dir(root)
        .args(["metadata", "--format-version", "1", "--locked", "--filter-platform", TARGET])
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !output.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&output.stderr)));
    }
    let meta: Value = serde_json::from_slice(&output.stdout).map_err(|e| format!("cargo metadata: bad JSON: {e}"))?;
    let crates = shipped(&meta)?;
    let text = render(&crates);
    std::fs::write(out, &text).map_err(|e| format!("writing {out}: {e}"))?;
    let missing = crates.iter().filter(|c| c.texts.is_empty()).count();
    println!("third-party: {} crates ({missing} without a licence file) -> {out} ({} KB)", crates.len(), text.len() / 1024);
    Ok(())
}

/// The third-party crates `APP` reaches through normal dependencies, with their licence texts.
fn shipped(meta: &Value) -> Result<Vec<Crate>, String> {
    let packages: HashMap<&str, &Value> = list(meta.get("packages")).iter().filter_map(|p| Some((p.get("id")?.as_str()?, p))).collect();
    let nodes: HashMap<&str, &Value> = list(meta.pointer("/resolve/nodes")).iter().filter_map(|n| Some((n.get("id")?.as_str()?, n))).collect();
    let app = packages
        .iter()
        .find(|(_, p)| str_of(p, "name") == APP && p.get("source").is_none_or(Value::is_null))
        .map(|(id, _)| *id)
        .ok_or_else(|| format!("no package `{APP}` in the workspace"))?;

    let mut seen = BTreeSet::from([app]);
    let mut queue = VecDeque::from([app]);
    while let Some(id) = queue.pop_front() {
        for dep in nodes.get(id).map(|n| list(n.get("deps"))).unwrap_or_default() {
            let Some(pkg) = dep.get("pkg").and_then(Value::as_str) else { continue };
            let normal = list(dep.get("dep_kinds")).iter().any(|k| k.get("kind").is_none_or(Value::is_null));
            let macro_crate = packages.get(pkg).is_some_and(|p| is_proc_macro(p));
            if normal && !macro_crate && seen.insert(pkg) {
                queue.push_back(pkg);
            }
        }
    }

    let mut crates: Vec<Crate> = seen
        .iter()
        .filter_map(|id| packages.get(id))
        .filter(|p| p.get("source").and_then(Value::as_str).is_some())
        .map(|p| {
            let source = str_of(p, "source");
            Crate {
                name: str_of(p, "name").to_owned(),
                version: str_of(p, "version").to_owned(),
                license: p.get("license").and_then(Value::as_str).unwrap_or("(none given)").to_owned(),
                source: shown_source(p, source),
                texts: texts_of(p, source),
            }
        })
        .collect();
    crates.sort();
    Ok(crates)
}

fn list(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn is_proc_macro(package: &Value) -> bool {
    let targets = list(package.get("targets"));
    !targets.is_empty() && targets.iter().all(|t| list(t.get("kind")).iter().any(|k| k.as_str() == Some("proc-macro")))
}

/// Where the crate comes from: its repository when it names one, else its source.
fn shown_source(package: &Value, source: &str) -> String {
    match package.get("repository").and_then(Value::as_str) {
        Some(repo) if !repo.is_empty() => repo.to_owned(),
        _ => source.split('#').next().unwrap_or(source).to_owned(),
    }
}

/// The licence and notice files of the crate's folder (and, from a git checkout, of the
/// repository's folders above it when the crate's has none).
fn texts_of(package: &Value, source: &str) -> Vec<String> {
    let Some(dir) = Path::new(str_of(package, "manifest_path")).parent() else { return Vec::new() };
    let mut files = licence_files(dir);
    if let Some(file) = package.get("license_file").and_then(Value::as_str) {
        let path = dir.join(file);
        if !files.contains(&path) {
            files.push(path);
        }
    }
    if files.is_empty() && source.starts_with("git+") {
        let mut up = dir.parent();
        // A git dependency sits in `~/.cargo/git/checkouts/<repo>/<rev>/...`: look no higher.
        while let Some(folder) = up {
            if folder.parent().and_then(Path::file_name).is_some_and(|n| n == "checkouts") {
                break;
            }
            files = licence_files(folder);
            if !files.is_empty() {
                break;
            }
            up = folder.parent();
        }
    }
    files.iter().filter_map(|f| std::fs::read(f).ok()).map(|b| tidy(&String::from_utf8_lossy(&b))).collect()
}

fn licence_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_uppercase()).unwrap_or_default();
            PREFIXES.iter().any(|prefix| name.starts_with(prefix))
        })
        .collect();
    files.sort();
    files
}

/// Line ends as `\n`, no byte-order mark, no trailing blank lines.
fn tidy(text: &str) -> String {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    text.trim_end().to_owned()
}

/// The file: a header, every crate on a line, then each distinct text once with its crates.
pub fn render(crates: &[Crate]) -> String {
    let mut by_text: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for c in crates {
        for t in &c.texts {
            by_text.entry(t.as_str()).or_default().push(format!("{} {}", c.name, c.version));
        }
    }
    // Most-shared texts first, so the common MIT and Apache texts open the list.
    let mut texts: Vec<(&str, Vec<String>)> = by_text.into_iter().collect();
    texts.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(b.0)));

    let mut out = String::new();
    out.push_str("Third-party software in Monun (monun.exe)\n\n");
    out.push_str("monun.exe is built from Monun's own sources (LICENSE-MIT, LICENSE-APACHE, NOTICE) and\n");
    out.push_str("the open-source crates below. Each is listed with its licence and source; the licence\n");
    out.push_str("and notice texts the crates ship follow, each text once, with the crates it covers.\n\n");
    out.push_str(&format!("== Crates ({}) ==\n\n", crates.len()));
    for c in crates {
        let note = if c.texts.is_empty() { "  [no licence file in the crate: see its source]" } else { "" };
        out.push_str(&format!("{} {}  {}  {}{note}\n", c.name, c.version, c.license, c.source));
    }
    out.push_str(&format!("\n== Licence and notice texts ({}) ==\n", texts.len()));
    for (i, (text, users)) in texts.iter().enumerate() {
        out.push_str(&format!("\n---- {}. Used by: {}\n\n{text}\n", i + 1, users.join(", ")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn krate(name: &str, texts: &[&str]) -> Crate {
        Crate {
            name: name.into(),
            version: "1.0.0".into(),
            license: "MIT".into(),
            source: format!("https://example.com/{name}"),
            texts: texts.iter().map(|t| (*t).to_owned()).collect(),
        }
    }

    #[test]
    fn a_shared_text_is_written_once_with_every_crate_it_covers() {
        let out = render(&[krate("a", &["MIT text"]), krate("b", &["MIT text", "B notice"]), krate("c", &[])]);
        assert_eq!(out.matches("MIT text").count(), 1, "{out}");
        assert!(out.contains("---- 1. Used by: a 1.0.0, b 1.0.0\n\nMIT text\n"), "{out}");
        assert!(out.contains("---- 2. Used by: b 1.0.0\n\nB notice\n"), "{out}");
        assert!(out.contains("c 1.0.0  MIT  https://example.com/c  [no licence file"), "{out}");
        assert!(out.contains("== Crates (3) =="), "{out}");
    }

    #[test]
    fn texts_are_tidied() {
        assert_eq!(tidy("\u{feff}line one\r\nline two\r\n\r\n"), "line one\nline two");
    }

    #[test]
    fn the_app_reaches_normal_dependencies_only() {
        let meta = serde_json::json!({
            "packages": [
                {"id": "app", "name": "monun", "version": "1.0.0", "source": null, "targets": [{"kind": ["bin"]}]},
                {"id": "lib", "name": "lib", "version": "1.0.0", "source": "registry+x", "license": "MIT",
                 "manifest_path": "/nowhere/lib/Cargo.toml", "targets": [{"kind": ["lib"]}]},
                {"id": "deep", "name": "deep", "version": "2.0.0", "source": "registry+x", "license": "Apache-2.0",
                 "manifest_path": "/nowhere/deep/Cargo.toml", "targets": [{"kind": ["lib"]}]},
                {"id": "mac", "name": "mac", "version": "1.0.0", "source": "registry+x",
                 "manifest_path": "/nowhere/mac/Cargo.toml", "targets": [{"kind": ["proc-macro"]}]},
                {"id": "build", "name": "build", "version": "1.0.0", "source": "registry+x",
                 "manifest_path": "/nowhere/build/Cargo.toml", "targets": [{"kind": ["lib"]}]},
                {"id": "dev", "name": "dev", "version": "1.0.0", "source": "registry+x",
                 "manifest_path": "/nowhere/dev/Cargo.toml", "targets": [{"kind": ["lib"]}]}
            ],
            "resolve": {"nodes": [
                {"id": "app", "deps": [
                    {"pkg": "lib", "dep_kinds": [{"kind": null}]},
                    {"pkg": "mac", "dep_kinds": [{"kind": null}]},
                    {"pkg": "build", "dep_kinds": [{"kind": "build"}]},
                    {"pkg": "dev", "dep_kinds": [{"kind": "dev"}]}
                ]},
                {"id": "lib", "deps": [{"pkg": "deep", "dep_kinds": [{"kind": null}]}]}
            ]}
        });
        let names: Vec<String> = shipped(&meta).unwrap().into_iter().map(|c| c.name).collect();
        assert_eq!(names, ["deep", "lib"]);
    }
}
