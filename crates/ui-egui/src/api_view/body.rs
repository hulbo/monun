//! A request's body, the ways Postman offers it, as a `.http` file writes each (so VS Code's REST
//! Client and IntelliJ's HTTP Client read them too):
//!
//! - none: no body;
//! - form-data: `Content-Type: multipart/form-data; boundary=…` and the parts written out, a file
//!   part's content `< path`;
//! - x-www-form-urlencoded: `Content-Type: application/x-www-form-urlencoded` and `a=1&b=2`;
//! - raw: the text, its kind (Text, JavaScript, JSON, HTML, XML) by its `Content-Type`, else by how
//!   it starts;
//! - binary: a body that is only `< path`: the file goes as it is;
//! - GraphQL: `X-Request-Type: GraphQL` (REST Client's mark), the query, a blank line and its
//!   variables (JSON); it goes as JSON `{"query": …, "variables": …}`.

use std::path::{Path, PathBuf};

use super::http_file::Request;

/// The boundary a form this tab writes uses.
pub const BOUNDARY: &str = "MonunFormBoundary";

/// How a request's body is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    None,
    FormData,
    UrlEncoded,
    Raw(RawKind),
    Binary,
    GraphQl,
}

/// What a raw body holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawKind {
    Text,
    JavaScript,
    Json,
    Html,
    Xml,
}

impl RawKind {
    pub const ALL: [RawKind; 5] = [RawKind::Text, RawKind::JavaScript, RawKind::Json, RawKind::Html, RawKind::Xml];

    pub fn label(self) -> &'static str {
        match self {
            RawKind::Text => "Text",
            RawKind::JavaScript => "JavaScript",
            RawKind::Json => "JSON",
            RawKind::Html => "HTML",
            RawKind::Xml => "XML",
        }
    }

    /// The `Content-Type` a body of the kind goes with.
    pub fn content_type(self) -> &'static str {
        match self {
            RawKind::Text => "text/plain",
            RawKind::JavaScript => "application/javascript",
            RawKind::Json => "application/json",
            RawKind::Html => "text/html",
            RawKind::Xml => "application/xml",
        }
    }

    /// The kind a `Content-Type` names, if it names one.
    fn of_type(ct: &str) -> Option<RawKind> {
        let ct = ct.to_ascii_lowercase();
        if ct.contains("json") {
            Some(RawKind::Json)
        } else if ct.contains("xml") {
            Some(RawKind::Xml)
        } else if ct.contains("html") {
            Some(RawKind::Html)
        } else if ct.contains("javascript") || ct.contains("ecmascript") {
            Some(RawKind::JavaScript)
        } else if ct.starts_with("text/") {
            Some(RawKind::Text)
        } else {
            None
        }
    }

    /// The kind a text reads as by how it starts: `{` or `[` JSON, an HTML page HTML, `<` XML.
    pub fn guess(text: &str) -> RawKind {
        let t = text.trim_start();
        let head = t.get(..t.len().min(15)).unwrap_or_default().to_ascii_lowercase();
        if t.starts_with('{') || t.starts_with('[') {
            RawKind::Json
        } else if head.starts_with("<!doctype html") || head.starts_with("<html") {
            RawKind::Html
        } else if t.starts_with('<') {
            RawKind::Xml
        } else {
            RawKind::Text
        }
    }
}

/// The value of the header `name` (regardless of case).
pub fn header<'a>(r: &'a Request, name: &str) -> Option<&'a str> {
    r.headers.iter().find(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

/// Sets the header `name` to `value` (in place, or added), or takes it out (`None`).
pub fn set_header(r: &mut Request, name: &str, value: Option<&str>) {
    match value {
        Some(v) => match r.headers.iter_mut().find(|(k, _)| k.trim().eq_ignore_ascii_case(name)) {
            Some(row) => row.1 = v.to_string(),
            None => r.headers.push((name.to_string(), v.to_string())),
        },
        None => r.headers.retain(|(k, _)| !k.trim().eq_ignore_ascii_case(name)),
    }
}

/// A body that is a file: `< path` alone (`<` alone: a file not chosen yet, its path empty).
pub fn file_ref(body: &str) -> Option<&str> {
    let t = body.trim();
    if t.contains('\n') {
        return None;
    }
    if t == "<" {
        return Some("");
    }
    t.strip_prefix("< ").map(str::trim)
}

/// A file a request sends, its path read: none chosen says so.
fn chosen(base: Option<&Path>, path: String) -> Result<PathBuf, String> {
    if path.trim().is_empty() { Err("Choose the file to send.".into()) } else { Ok(resolve(base, path.trim())) }
}

/// How `r`'s body is written.
pub fn kind(r: &Request) -> BodyKind {
    if header(r, "X-Request-Type").is_some_and(|v| v.trim().eq_ignore_ascii_case("graphql")) {
        return BodyKind::GraphQl;
    }
    let ct = header(r, "Content-Type").unwrap_or_default().trim().to_ascii_lowercase();
    if ct.starts_with("multipart/form-data") {
        return BodyKind::FormData;
    }
    if ct.contains("x-www-form-urlencoded") {
        return BodyKind::UrlEncoded;
    }
    if r.body.trim().is_empty() && ct.is_empty() {
        return BodyKind::None;
    }
    if file_ref(&r.body).is_some() {
        return BodyKind::Binary;
    }
    BodyKind::Raw(RawKind::of_type(&ct).unwrap_or_else(|| RawKind::guess(&r.body)))
}

/// The boundary a `Content-Type` names (`boundary=…`, quoted or not).
pub fn boundary(content_type: &str) -> Option<String> {
    content_type.split(';').find_map(|p| {
        let (k, v) = p.split_once('=')?;
        k.trim().eq_ignore_ascii_case("boundary").then(|| v.trim().trim_matches('"').to_string()).filter(|b| !b.is_empty())
    })
}

/// A form-data field: a name and a text, or a file (its path, and the type it goes as).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub value: String,
    pub file: bool,
    pub content_type: String,
}

/// What a part's `Content-Disposition` says: its name, and its file name when it is a file.
fn disposition(line: &str) -> (String, Option<String>) {
    let param = |key: &str| {
        line.split(';').find_map(|p| {
            let (k, v) = p.split_once('=')?;
            (k.trim().eq_ignore_ascii_case(key)).then(|| v.trim().trim_matches('"').to_string())
        })
    };
    (param("name").unwrap_or_default(), param("filename"))
}

/// The fields of a form-data body written with `boundary`.
pub fn fields(body: &str, boundary: &str) -> Vec<Field> {
    let delim = format!("--{boundary}");
    let mut out = Vec::new();
    for chunk in body.split(delim.as_str()).skip(1) {
        if chunk.starts_with("--") {
            break;
        }
        let chunk = chunk.strip_prefix('\n').unwrap_or(chunk);
        let (head, content) = chunk.split_once("\n\n").unwrap_or((chunk, ""));
        let mut field = Field::default();
        let mut filename = None;
        for line in head.lines() {
            let Some((k, v)) = line.split_once(':') else { continue };
            if k.trim().eq_ignore_ascii_case("content-disposition") {
                let (name, file) = disposition(v);
                field.name = name;
                filename = file;
            } else if k.trim().eq_ignore_ascii_case("content-type") {
                field.content_type = v.trim().to_string();
            }
        }
        let content = content.strip_suffix('\n').unwrap_or(content);
        match (filename, file_ref(content)) {
            (_, Some(path)) => {
                field.file = true;
                field.value = path.to_string();
            }
            (Some(_), None) => {
                field.file = true;
                field.value = String::new();
            }
            (None, None) => field.value = content.to_string(),
        }
        out.push(field);
    }
    out
}

/// A form-data body of `fields`, written with `boundary`.
pub fn render_fields(fields: &[Field], boundary: &str) -> String {
    let quote = |s: &str| s.replace('"', "%22");
    let mut s = String::new();
    for f in fields {
        s.push_str(&format!("--{boundary}\nContent-Disposition: form-data; name=\"{}\"", quote(&f.name)));
        if f.file {
            let name = Path::new(&f.value).file_name().and_then(|n| n.to_str()).unwrap_or("file");
            s.push_str(&format!("; filename=\"{}\"", quote(name)));
        }
        s.push('\n');
        if f.file && !f.content_type.trim().is_empty() {
            s.push_str(&format!("Content-Type: {}\n", f.content_type.trim()));
        }
        s.push('\n');
        if f.file {
            s.push_str(&format!("< {}\n", f.value));
        } else {
            s.push_str(&f.value);
            s.push('\n');
        }
    }
    if !fields.is_empty() {
        s.push_str(&format!("--{boundary}--"));
    }
    s
}

/// The pairs of an urlencoded body (a line a pair, or `&` between them), an `&` or `=` written
/// `%26` and `%3D` read back as they are typed.
pub fn pairs(body: &str) -> Vec<(String, String)> {
    let joined: String = body.lines().map(str::trim).collect();
    if joined.is_empty() {
        return Vec::new();
    }
    let typed = |s: &str| s.replace("%26", "&").replace("%3D", "=");
    joined.split('&').map(|p| p.split_once('=').map_or((typed(p), String::new()), |(k, v)| (typed(k), typed(v)))).collect()
}

/// An urlencoded body of `pairs`: as typed but for an `&` (and a name's `=`), which would split it
/// (the rest is encoded when it goes).
pub fn render_pairs(pairs: &[(String, String)]) -> String {
    let safe = |s: &str| s.replace('&', "%26");
    pairs.iter().map(|(k, v)| format!("{}={}", safe(k).replace('=', "%3D"), safe(v))).collect::<Vec<_>>().join("&")
}

/// `s` encoded for a form: letters, digits and `-_.~` as they are, a space `+`, what is already
/// encoded (`%41`) left so, the rest `%XX`.
pub fn form_encode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len() * 3);
    let hex = |c: u8| c.is_ascii_hexdigit();
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(c as char),
            b' ' => out.push('+'),
            b'%' if b.get(i + 1).copied().is_some_and(hex) && b.get(i + 2).copied().is_some_and(hex) => out.push('%'),
            _ => out.push_str(&format!("%{c:02X}")),
        }
        i += 1;
    }
    out
}

/// A GraphQL body's query and variables: the variables are the JSON after the last blank line.
pub fn graphql(body: &str) -> (String, String) {
    let mut at = body.len();
    while let Some(i) = body.get(..at).and_then(|b| b.rfind("\n\n")) {
        let rest = body.get(i + 2..).unwrap_or_default().trim();
        if rest.starts_with('{') && serde_json::from_str::<serde::de::IgnoredAny>(rest).is_ok() {
            return (body.get(..i).unwrap_or_default().trim_end().to_string(), rest.to_string());
        }
        at = i;
    }
    (body.trim_end().to_string(), String::new())
}

/// A GraphQL body of `query` and `variables`.
pub fn render_graphql(query: &str, variables: &str) -> String {
    let mut s = query.trim_end().to_string();
    if !variables.trim().is_empty() {
        s.push_str("\n\n");
        s.push_str(variables.trim());
    }
    s
}

/// What goes as a request's body, its variables read; files are read on the way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    Empty,
    Text(String),
    File(PathBuf),
    Form { boundary: String, parts: Vec<Part> },
}

/// A part of a form-data body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub value: PartValue,
    pub content_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PartValue {
    Text(String),
    File(PathBuf),
}

/// `path` as a request names it, relative to the folder `base` of its file.
pub fn resolve(base: Option<&Path>, path: &str) -> PathBuf {
    let p = Path::new(path);
    match base {
        Some(b) if p.is_relative() => b.join(p),
        _ => p.to_path_buf(),
    }
}

/// `path` as a request writes it: relative to the folder `base` of its file when inside it.
pub fn written(base: Option<&Path>, path: &str) -> String {
    let p = Path::new(path);
    match base.and_then(|b| p.strip_prefix(b).ok()) {
        Some(rest) => format!("./{}", rest.to_string_lossy().replace('\\', "/")),
        None => path.to_string(),
    }
}

/// What `r` sends (its variables read by `read`) and the headers it goes with: a form's boundary
/// named, GraphQL's mark taken off, the type of a raw body that names none added (as Postman adds
/// it). Paths are the file's folder's (`base`).
pub fn payload(r: &Request, read: &mut dyn FnMut(&str) -> String, base: Option<&Path>) -> Result<(Payload, Vec<(String, String)>), String> {
    let mut headers: Vec<(String, String)> = r.headers.clone();
    let mut put = |name: &str, value: Option<&str>| {
        let mut req = Request { headers: std::mem::take(&mut headers), ..Request::default() };
        set_header(&mut req, name, value);
        headers = req.headers;
    };
    let body = match kind(r) {
        BodyKind::None => Payload::Empty,
        BodyKind::Raw(k) => {
            if header(r, "Content-Type").is_none() {
                put("Content-Type", Some(k.content_type()));
            }
            Payload::Text(read(&r.body))
        }
        BodyKind::UrlEncoded => {
            let pairs: Vec<String> = pairs(&r.body).iter().map(|(k, v)| format!("{}={}", form_encode(&read(k)), form_encode(&read(v)))).collect();
            Payload::Text(pairs.join("&"))
        }
        BodyKind::Binary => {
            if header(r, "Content-Type").is_none() {
                put("Content-Type", Some("application/octet-stream"));
            }
            Payload::File(chosen(base, read(file_ref(&r.body).unwrap_or_default()))?)
        }
        BodyKind::FormData => {
            let ct = header(r, "Content-Type").unwrap_or_default();
            let named = boundary(ct);
            let boundary = named.clone().unwrap_or_else(|| BOUNDARY.to_string());
            if named.is_none() {
                put("Content-Type", Some(&format!("multipart/form-data; boundary={boundary}")));
            }
            let mut parts = Vec::new();
            for f in fields(&r.body, &boundary) {
                let value = if f.file { PartValue::File(chosen(base, read(&f.value))?) } else { PartValue::Text(read(&f.value)) };
                parts.push(Part { name: read(&f.name), value, content_type: f.content_type });
            }
            Payload::Form { boundary, parts }
        }
        BodyKind::GraphQl => {
            put("X-Request-Type", None);
            if header(r, "Content-Type").is_none() {
                put("Content-Type", Some("application/json"));
            }
            let (query, variables) = graphql(&r.body);
            let query = read(&query);
            let variables = read(&variables);
            let mut json = serde_json::json!({ "query": query });
            if !variables.trim().is_empty() {
                let v: serde_json::Value = serde_json::from_str(&variables).map_err(|e| format!("The GraphQL variables aren't valid JSON.\n{e}"))?;
                if let Some(obj) = json.as_object_mut() {
                    obj.insert("variables".into(), v);
                }
            }
            Payload::Text(json.to_string())
        }
    };
    Ok((body, headers))
}

/// The type a file goes as in a form, by its extension.
pub fn file_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("json") => "application/json",
        Some("xml") => "application/xml",
        Some("txt" | "csv" | "log") => "text/plain",
        Some("html" | "htm") => "text/html",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("pdf") => "application/pdf",
        Some("zip") => "application/zip",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
}

/// The bytes of `payload`: files read, a form's parts each between its boundary lines (CRLF).
pub fn bytes(payload: &Payload) -> Result<Vec<u8>, String> {
    let read = |p: &Path| std::fs::read(p).map_err(|e| format!("Couldn't read the file to send.\n{}: {e}", p.display()));
    match payload {
        Payload::Empty => Ok(Vec::new()),
        Payload::Text(t) => Ok(t.clone().into_bytes()),
        Payload::File(p) => read(p),
        Payload::Form { boundary, parts } => {
            let mut out = Vec::new();
            for part in parts {
                out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
                let name = part.name.replace('"', "%22");
                match &part.value {
                    PartValue::Text(t) => {
                        out.extend_from_slice(format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes());
                        out.extend_from_slice(t.as_bytes());
                    }
                    PartValue::File(p) => {
                        let file = p.file_name().and_then(|n| n.to_str()).unwrap_or("file").replace('"', "%22");
                        let ct = if part.content_type.trim().is_empty() { file_type(p) } else { part.content_type.trim() };
                        out.extend_from_slice(
                            format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{file}\"\r\nContent-Type: {ct}\r\n\r\n").as_bytes(),
                        );
                        out.extend_from_slice(&read(p)?);
                    }
                }
                out.extend_from_slice(b"\r\n");
            }
            out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(headers: &[(&str, &str)], body: &str) -> Request {
        Request {
            method: "POST".into(),
            url: "https://x.test/".into(),
            headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            body: body.into(),
            ..Request::default()
        }
    }

    #[test]
    fn a_body_reads_as_the_way_it_is_written() {
        assert_eq!(kind(&req(&[], "")), BodyKind::None);
        assert_eq!(kind(&req(&[("Content-Type", "multipart/form-data; boundary=b")], "")), BodyKind::FormData);
        assert_eq!(kind(&req(&[("content-type", "application/x-www-form-urlencoded")], "a=1")), BodyKind::UrlEncoded);
        assert_eq!(kind(&req(&[("Content-Type", "application/json")], "")), BodyKind::Raw(RawKind::Json));
        assert_eq!(kind(&req(&[], "<Envelope><Body/></Envelope>")), BodyKind::Raw(RawKind::Xml), "by how it starts");
        assert_eq!(kind(&req(&[], "{\"a\":1}")), BodyKind::Raw(RawKind::Json));
        assert_eq!(kind(&req(&[], "<!DOCTYPE html><p>")), BodyKind::Raw(RawKind::Html));
        assert_eq!(kind(&req(&[("Content-Type", "text/xml; charset=utf-8")], "x")), BodyKind::Raw(RawKind::Xml));
        assert_eq!(kind(&req(&[], "< ./photo.png")), BodyKind::Binary);
        assert_eq!(kind(&req(&[("X-REQUEST-TYPE", "GraphQL")], "query { a }")), BodyKind::GraphQl);
    }

    #[test]
    fn a_form_is_written_and_read_back() {
        let fields_in = vec![
            Field { name: "title".into(), value: "모눈 테스트 ".into(), ..Field::default() },
            Field { name: "photo".into(), value: "./img/a.png".into(), file: true, content_type: "image/png".into() },
        ];
        let body = render_fields(&fields_in, BOUNDARY);
        assert!(
            body.contains("Content-Disposition: form-data; name=\"photo\"; filename=\"a.png\"\nContent-Type: image/png\n\n< ./img/a.png\n"),
            "{body}"
        );
        assert_eq!(fields(&body, BOUNDARY), fields_in);
        assert_eq!(boundary("multipart/form-data; boundary=\"abc\""), Some("abc".into()));
        assert_eq!(pairs("a=1&b=\n&c=3"), [("a".into(), "1".into()), ("b".into(), String::new()), ("c".into(), "3".into())]);
        assert_eq!(render_pairs(&pairs("a=1&b=2")), "a=1&b=2");
        let typed = vec![("q=x".to_string(), "A&B".to_string())];
        assert_eq!(render_pairs(&typed), "q%3Dx=A%26B");
        assert_eq!(pairs(&render_pairs(&typed)), typed, "read back as typed");
        assert_eq!(form_encode("김 a&b=%41"), "%EA%B9%80+a%26b%3D%41");
    }

    #[test]
    fn graphql_keeps_its_variables_after_a_blank_line() {
        let body = render_graphql("query ($id: ID!) {\n\n  user(id: $id) { name }\n}", "{\"id\": \"7\"}");
        assert_eq!(graphql(&body), ("query ($id: ID!) {\n\n  user(id: $id) { name }\n}".into(), "{\"id\": \"7\"}".into()));
        assert_eq!(graphql("{ a }"), ("{ a }".into(), String::new()), "a query alone has no variables");
    }

    #[test]
    fn what_goes_is_read_and_its_headers_named() {
        let mut read = |s: &str| s.replace("{{Id}}", "7");
        let base = std::env::temp_dir().join(format!("monun-body-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&base);
        std::fs::write(base.join("a.txt"), "A").unwrap();
        // A form: its boundary named, its file read, CRLF between.
        let form = req(
            &[("Content-Type", "multipart/form-data")],
            &render_fields(
                &[
                    Field { name: "id".into(), value: "{{Id}}".into(), ..Field::default() },
                    Field { name: "f".into(), value: "./a.txt".into(), file: true, ..Field::default() },
                ],
                BOUNDARY,
            ),
        );
        let (p, headers) = payload(&form, &mut read, Some(&base)).unwrap();
        assert_eq!(headers, [("Content-Type".to_string(), format!("multipart/form-data; boundary={BOUNDARY}"))]);
        let sent = String::from_utf8(bytes(&p).unwrap()).unwrap();
        assert_eq!(
            sent,
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"id\"\r\n\r\n7\r\n--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"f\"; filename=\"a.txt\"\r\nContent-Type: text/plain\r\n\r\nA\r\n--{BOUNDARY}--\r\n"
            )
        );
        // Urlencoded: encoded as it goes.
        let (p, _) = payload(&req(&[("Content-Type", "application/x-www-form-urlencoded")], "name=김 {{Id}}&x=1"), &mut read, None).unwrap();
        assert_eq!(p, Payload::Text("name=%EA%B9%80+7&x=1".into()));
        // A raw body naming no type goes with the one it reads as.
        let (p, headers) = payload(&req(&[], "<a>{{Id}}</a>"), &mut read, None).unwrap();
        assert_eq!((p, headers), (Payload::Text("<a>7</a>".into()), vec![("Content-Type".to_string(), "application/xml".to_string())]));
        // GraphQL: as JSON, its mark off.
        let (p, headers) = payload(&req(&[("X-Request-Type", "GraphQL")], "{ user(id: {{Id}}) { name } }\n\n{\"a\": 1}"), &mut read, None).unwrap();
        assert_eq!(p, Payload::Text("{\"query\":\"{ user(id: 7) { name } }\",\"variables\":{\"a\":1}}".into()));
        assert_eq!(headers, [("Content-Type".to_string(), "application/json".to_string())]);
        // A file: read as it goes; one missing says so.
        let (p, _) = payload(&req(&[], "< ./a.txt"), &mut read, Some(&base)).unwrap();
        assert_eq!(bytes(&p).unwrap(), b"A");
        assert!(bytes(&Payload::File(base.join("none.bin"))).unwrap_err().starts_with("Couldn't read the file to send."));
        assert_eq!(written(Some(&base), &base.join("sub").join("b.png").to_string_lossy()), "./sub/b.png");
        let _ = std::fs::remove_dir_all(&base);
    }
}
