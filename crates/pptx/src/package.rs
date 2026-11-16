//! The package: the zip's parts, their relationships, and small helpers to read their XML (by
//! local names, so prefixes and the transitional or strict namespaces don't matter).

use std::collections::HashMap;
use std::io::{Cursor, Read};

pub type Node<'a, 'i> = roxmltree::Node<'a, 'i>;

/// The most of one part read: larger ones (a video) are left out.
const PART_MAX: u64 = 256 * 1024 * 1024;

/// Sound and video, never shown: left out of the parts read (a slide shows a video's poster).
const LEFT_OUT: [&str; 10] = [".mp4", ".m4v", ".mov", ".wmv", ".avi", ".mpg", ".mp3", ".wav", ".m4a", ".wma"];

/// CSS pixels to a point.
pub const PX: f64 = 4.0 / 3.0;

/// The parts of a package, by their names (lower case: part names don't go by case).
pub struct Package {
    parts: HashMap<String, Vec<u8>>,
}

impl Package {
    /// The package in `bytes`, or `None` when they aren't a zip package.
    pub fn read(bytes: &[u8]) -> Option<Package> {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
        let mut parts = HashMap::new();
        for i in 0..zip.len() {
            let Ok(entry) = zip.by_index(i) else { continue };
            let name = entry.name().trim_start_matches('/').to_ascii_lowercase();
            if entry.is_dir() || LEFT_OUT.iter().any(|x| name.ends_with(x)) {
                continue;
            }
            let mut data = Vec::new();
            if entry.take(PART_MAX).read_to_end(&mut data).is_ok() {
                parts.insert(name, data);
            }
        }
        Some(Package { parts })
    }

    pub fn bytes(&self, part: &str) -> Option<&[u8]> {
        self.parts.get(&part.to_ascii_lowercase()).map(Vec::as_slice)
    }

    /// A part's text (XML), without a byte order mark.
    pub fn text(&self, part: &str) -> Option<String> {
        let text = String::from_utf8_lossy(self.bytes(part)?);
        Some(text.trim_start_matches('\u{feff}').to_string())
    }

    /// The relationships of `part` (`""`: the package's own).
    pub fn rels(&self, part: &str) -> Rels {
        let file = match part.rsplit_once('/') {
            Some((dir, name)) => format!("{dir}/_rels/{name}.rels"),
            None => format!("_rels/{part}.rels"),
        };
        self.text(&file).map(|xml| Rels::parse(part, &xml)).unwrap_or_default()
    }
}

/// A part's relationships, by their ids.
#[derive(Clone, Debug, Default)]
pub struct Rels {
    by_id: HashMap<String, Rel>,
}

/// A relationship: its kind (the last part of its type, `slideLayout`, `image`…) and the part it
/// names (or an address outside the package).
#[derive(Clone, Debug)]
pub struct Rel {
    pub kind: String,
    pub target: String,
    pub external: bool,
}

impl Rels {
    fn parse(part: &str, xml: &str) -> Rels {
        let Some(doc) = parse(xml) else { return Rels::default() };
        let by_id = elems(doc.root_element())
            .filter_map(|r| {
                let id = attr(r, "Id")?;
                let target = attr(r, "Target")?;
                let kind = attr(r, "Type").and_then(|t| t.rsplit('/').next()).unwrap_or_default().to_string();
                let external = attr(r, "TargetMode") == Some("External");
                let target = if external { target.to_string() } else { resolve(part, &decode(target)) };
                Some((id.to_string(), Rel { kind, target, external }))
            })
            .collect();
        Rels { by_id }
    }

    /// The part relationship `id` names.
    pub fn part(&self, id: &str) -> Option<&str> {
        self.by_id.get(id).filter(|r| !r.external).map(|r| r.target.as_str())
    }

    /// The part of the first relationship of `kind` (by its part's name, for a stable choice).
    pub fn first(&self, kind: &str) -> Option<&str> {
        self.by_id.values().filter(|r| r.kind == kind && !r.external).map(|r| r.target.as_str()).min()
    }
}

/// The part `target` names, relative to `part`'s folder (or the package's root, with a `/`).
pub fn resolve(part: &str, target: &str) -> String {
    if let Some(root) = target.strip_prefix('/') {
        return root.to_string();
    }
    let mut path: Vec<&str> = part.rsplit_once('/').map(|(dir, _)| dir.split('/').collect()).unwrap_or_default();
    for step in target.split('/') {
        match step {
            "" | "." => {}
            ".." => {
                path.pop();
            }
            name => path.push(name),
        }
    }
    path.join("/")
}

/// `%20` and the like in a target, as the part's name has them plain.
fn decode(target: &str) -> String {
    let bytes = target.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (b, hex) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| target.to_string())
}

pub fn parse(xml: &str) -> Option<roxmltree::Document<'_>> {
    let options = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    roxmltree::Document::parse_with_options(xml, options).ok()
}

/// `n`'s child elements.
pub fn elems<'a, 'i>(n: Node<'a, 'i>) -> impl Iterator<Item = Node<'a, 'i>> {
    n.children().filter(Node::is_element)
}

/// `n`'s first child element called `name`.
pub fn kid<'a, 'i>(n: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    elems(n).find(|c| c.tag_name().name() == name)
}

/// `n`'s child elements called `name`.
pub fn kids<'a, 'i: 'a>(n: Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    elems(n).filter(move |c| c.tag_name().name() == name)
}

/// The element down `path` from `n`.
pub fn at<'a, 'i>(n: Node<'a, 'i>, path: &[&str]) -> Option<Node<'a, 'i>> {
    path.iter().try_fold(n, |n, step| kid(n, step))
}

pub fn attr<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attribute(name)
}

/// A relationship attribute (`r:embed`, `r:id`…).
pub fn rid<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attributes().find(|a| a.name() == name && a.namespace().is_some_and(|ns| ns.ends_with("relationships"))).map(|a| a.value())
}

pub fn int(n: Node, name: &str) -> Option<i64> {
    attr(n, name)?.trim().parse().ok()
}

pub fn flag(n: Node, name: &str) -> Option<bool> {
    match attr(n, name)? {
        "1" | "true" | "on" => Some(true),
        "0" | "false" | "off" => Some(false),
        _ => None,
    }
}

/// English Metric Units in points.
pub fn pt(emu: i64) -> f64 {
    emu as f64 / 12_700.0
}

/// `s` as HTML text, or inside an attribute in double quotes.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}
