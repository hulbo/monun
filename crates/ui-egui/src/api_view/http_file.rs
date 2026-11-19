//! `.http` request files, as VS Code's REST Client and IntelliJ's HTTP Client write them: requests
//! separated by `###` lines (the rest of the line names the request), each with comments (`#`,
//! `//`), `@name = value` variables, a request line (`POST https://…`, the method `GET` when left
//! out), headers (`Name: value`), a blank line and the body. Text before the first request holding
//! only comments and variables is the file's: its variables reach every request.
//!
//! A request edited in the tab is written back alone: the rest of the file stays as it was.

use std::ops::Range;

/// The methods a request line may start with.
pub const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

/// A request of a file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Request {
    /// What follows `###` on the line that starts it.
    pub name: String,
    /// Comment lines before the request line, as written.
    pub notes: Vec<String>,
    /// Its `@name = value` variables.
    pub vars: Vec<(String, String)>,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// A part of a file: a request, or text holding only comments and variables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    /// Its text's bytes in the file.
    pub range: Range<usize>,
    pub request: Request,
    /// Whether it has a request line.
    pub is_request: bool,
}

/// Whether `line` starts a request: `###` and anything after it.
fn starts_part(line: &str) -> bool {
    line.trim_start().starts_with("###")
}

/// The parts of a file's text, in order: each starts at a `###` line (the first at the start).
pub fn parts(text: &str) -> Vec<Part> {
    let mut starts = vec![0];
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if at > 0 && starts_part(line) {
            starts.push(at);
        }
        at += line.len();
    }
    let mut out = Vec::new();
    for (i, start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(text.len());
        let Some(slice) = text.get(*start..end) else { continue };
        let (request, is_request) = parse(slice);
        if slice.trim().is_empty() && !out.is_empty() {
            continue;
        }
        out.push(Part { range: *start..end, request, is_request });
    }
    out
}

/// The requests of a file (the parts with a request line).
pub fn requests(text: &str) -> Vec<Part> {
    parts(text).into_iter().filter(|p| p.is_request).collect()
}

/// The variables of a file's parts that aren't requests: those every request of it reads.
pub fn file_vars(text: &str) -> Vec<(String, String)> {
    parts(text).into_iter().filter(|p| !p.is_request).flat_map(|p| p.request.vars).collect()
}

/// Where a part is in its reading.
#[derive(PartialEq)]
enum Stage {
    Before,
    Headers,
    Body,
}

/// A part's request, and whether it has a request line.
pub fn parse(text: &str) -> (Request, bool) {
    let mut r = Request::default();
    let mut stage = Stage::Before;
    let mut body: Vec<&str> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        match stage {
            Stage::Before => {
                if starts_part(line) {
                    r.name = t.trim_start_matches('#').trim().to_string();
                } else if t.is_empty() {
                } else if t.starts_with('#') || t.starts_with("//") {
                    r.notes.push(line.to_string());
                } else if let Some(var) = t.strip_prefix('@')
                    && let Some((name, value)) = var.split_once('=')
                {
                    r.vars.push((name.trim().to_string(), value.trim().to_string()));
                } else {
                    let (method, url) = request_line(t);
                    r.method = method;
                    r.url = url;
                    stage = Stage::Headers;
                }
            }
            Stage::Headers => {
                if t.is_empty() {
                    stage = Stage::Body;
                } else if line.starts_with(char::is_whitespace) && (t.starts_with('?') || t.starts_with('&')) {
                    // The address goes on, a line a parameter.
                    r.url.push_str(t);
                } else if t.starts_with('#') || t.starts_with("//") {
                } else if let Some((name, value)) = t.split_once(':') {
                    r.headers.push((name.trim().to_string(), value.trim().to_string()));
                } else {
                    stage = Stage::Body;
                    body.push(line);
                }
            }
            Stage::Body => body.push(line),
        }
    }
    let is_request = stage != Stage::Before;
    r.body = body.join("\n").trim_end().to_string();
    (r, is_request)
}

/// A request line's method (upper case) and address: `GET` when it names none, and the protocol
/// (`HTTP/1.1`) at its end left off.
fn request_line(line: &str) -> (String, String) {
    let (first, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let (method, url) = if METHODS.iter().any(|m| m.eq_ignore_ascii_case(first)) && !rest.trim().is_empty() {
        (first.to_ascii_uppercase(), rest.trim())
    } else {
        ("GET".to_string(), line.trim())
    };
    let url = match url.rsplit_once(char::is_whitespace) {
        Some((head, proto)) if proto.starts_with("HTTP/") => head.trim(),
        _ => url,
    };
    (method, url.to_string())
}

/// A request's text, as this tab writes it.
pub fn render(r: &Request) -> String {
    let mut s = String::from("###");
    if !r.name.is_empty() {
        s.push(' ');
        s.push_str(&r.name);
    }
    s.push('\n');
    for n in &r.notes {
        s.push_str(n);
        s.push('\n');
    }
    for (k, v) in &r.vars {
        s.push_str(&format!("@{k} = {v}\n"));
    }
    let method = if r.method.is_empty() { "GET" } else { &r.method };
    s.push_str(&format!("{method} {}\n", r.url));
    for (k, v) in &r.headers {
        s.push_str(&format!("{k}: {v}\n"));
    }
    if !r.body.trim().is_empty() {
        s.push('\n');
        s.push_str(&r.body);
        s.push('\n');
    }
    s
}

/// `text` with the part `range` written as `r`: the rest stays as it was, a blank line before the
/// part after it.
pub fn replace(text: &str, range: Range<usize>, r: &Request) -> String {
    let (Some(before), Some(after)) = (text.get(..range.start), text.get(range.end..)) else { return text.to_string() };
    let mut out = String::with_capacity(text.len() + 64);
    out.push_str(before);
    if !before.is_empty() && !before.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&render(r));
    if !after.is_empty() {
        out.push('\n');
        out.push_str(after);
    }
    out
}

/// A new request file's text.
pub fn template(name: &str) -> String {
    render(&Request { name: name.to_string(), method: "GET".into(), url: "https://{{Domain}}/".into(), ..Request::default() })
}

/// `text` with every `{{name}}` read by `lookup` (`{{$timestamp}}`: the seconds since 1970, from
/// `now_ms`); the names with no value, left as they were, come back too.
pub fn substitute(text: &str, lookup: &dyn Fn(&str) -> Option<String>, now_ms: f64) -> (String, Vec<String>) {
    let mut out = String::with_capacity(text.len());
    let mut missing = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        let Some(len) = rest.get(open + 2..).and_then(|r| r.find("}}")) else { break };
        let (Some(head), Some(inner), Some(tail)) = (rest.get(..open), rest.get(open + 2..open + 2 + len), rest.get(open + 4 + len..)) else {
            break;
        };
        out.push_str(head);
        let name = inner.trim();
        let value = match name {
            "$timestamp" => Some(format!("{}", (now_ms / 1000.0).floor() as i64)),
            "$isoTimestamp" => Some(iso_time(now_ms)),
            _ => lookup(name),
        };
        match value {
            Some(v) => out.push_str(&v),
            None => {
                if !missing.iter().any(|m| m == name) {
                    missing.push(name.to_string());
                }
                out.push_str("{{");
                out.push_str(inner);
                out.push_str("}}");
            }
        }
        rest = tail;
    }
    out.push_str(rest);
    (out, missing)
}

/// The names `{{…}}` in `text`, in order, each once.
pub fn names(text: &str) -> Vec<String> {
    let (_, missing) = substitute(text, &|_| None, 0.0);
    missing.into_iter().filter(|n| !n.starts_with('$')).collect()
}

/// `ms` since 1970 as `2026-11-19T05:06:20Z`.
fn iso_time(ms: f64) -> String {
    let secs = (ms / 1000.0).floor() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// `text` laid out with a value a line and two spaces an indent, when it is JSON; the keys stay in
/// their order.
pub fn pretty_json(text: &str) -> Option<String> {
    let t = text.trim();
    if !(t.starts_with('{') || t.starts_with('[')) || serde_json::from_str::<serde::de::IgnoredAny>(t).is_err() {
        return None;
    }
    let mut out = String::with_capacity(t.len() * 2);
    let mut depth = 0usize;
    let (mut in_str, mut escaped) = (false, false);
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        out.push_str(&"  ".repeat(depth));
    };
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
            }
            '{' | '[' => {
                out.push(c);
                // An empty object or array stays on its line.
                while chars.peek().is_some_and(|n| n.is_whitespace()) {
                    chars.next();
                }
                if chars.peek().is_some_and(|n| *n == '}' || *n == ']') {
                    if let Some(close) = chars.next() {
                        out.push(close);
                    }
                } else {
                    depth += 1;
                    newline(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(c);
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "@Domain = dev.example.com\n\n### Approve\n# the card's approval\nPOST https://{{Domain}}/pay/v1/{{CustNo}} HTTP/1.1\nContent-Type: application/json\nAuthorization: Bearer {{Token}}\n\n{\n  \"amount\": 1000\n}\n\n### Cancel\nDELETE https://{{Domain}}/pay/v1/1\n";

    #[test]
    fn a_file_reads_as_its_requests_and_its_own_variables() {
        let reqs = requests(FILE);
        assert_eq!(reqs.len(), 2);
        let r = &reqs[0].request;
        assert_eq!((r.name.as_str(), r.method.as_str(), r.url.as_str()), ("Approve", "POST", "https://{{Domain}}/pay/v1/{{CustNo}}"));
        assert_eq!(r.notes, ["# the card's approval"]);
        assert_eq!(r.headers, [("Content-Type".into(), "application/json".into()), ("Authorization".into(), "Bearer {{Token}}".into())]);
        assert_eq!(r.body, "{\n  \"amount\": 1000\n}");
        assert_eq!((reqs[1].request.method.as_str(), reqs[1].request.body.as_str()), ("DELETE", ""));
        assert_eq!(file_vars(FILE), [("Domain".to_string(), "dev.example.com".to_string())]);
        // A bare address is a GET; a parameter a line goes on the address.
        let (r, is) = parse("https://x.test/a\n    ?b=1\n    &c=2\nAccept: */*\n");
        assert!(is);
        assert_eq!((r.method.as_str(), r.url.as_str(), r.headers.len()), ("GET", "https://x.test/a?b=1&c=2", 1));
        assert!(!parse("# only a note\n@a = 1\n").1);
    }

    #[test]
    fn a_request_written_back_leaves_the_rest_of_the_file_as_it_was() {
        let reqs = requests(FILE);
        let mut r = reqs[1].request.clone();
        r.headers.push(("Accept".into(), "*/*".into()));
        let text = replace(FILE, reqs[1].range.clone(), &r);
        assert!(text.starts_with(&FILE[..reqs[1].range.start]), "{text}");
        assert!(text.ends_with("### Cancel\nDELETE https://{{Domain}}/pay/v1/1\nAccept: */*\n"), "{text}");
        let first = requests(&text)[0].request.clone();
        assert_eq!(first, reqs[0].request);
        // Written and read again: the same request.
        let again = replace(&text, requests(&text)[0].range.clone(), &first);
        assert_eq!(requests(&again)[0].request, reqs[0].request);
        assert_eq!(requests(&again)[1].request, r);
        assert_eq!(parse(&template("New")).0.url, "https://{{Domain}}/");
    }

    #[test]
    fn variables_read_their_values_and_those_without_one_are_named() {
        let lookup = |n: &str| (n == "Domain").then(|| "dev.example.com".to_string());
        let (s, missing) = substitute("https://{{ Domain }}/v1/{{CustNo}}/{{CustNo}}?t={{$timestamp}}", &lookup, 1_700_000_000_500.0);
        assert_eq!(s, "https://dev.example.com/v1/{{CustNo}}/{{CustNo}}?t=1700000000");
        assert_eq!(missing, ["CustNo"]);
        assert_eq!(substitute("{{$isoTimestamp}}", &|_| None, 1_794_536_109_000.0).0, "2026-11-13T02:15:09Z");
        assert_eq!(substitute("a {{ b", &|_| None, 0.0).0, "a {{ b", "an unclosed brace stays");
        assert_eq!(names("{{A}}/{{$timestamp}}/{{B}}/{{A}}"), ["A", "B"]);
    }

    #[test]
    fn json_is_laid_out_in_its_own_order() {
        let pretty = pretty_json(r#"{"z":1,"a":[1,2,{}],"s":"x, y: {z}","e":[] }"#).unwrap();
        assert_eq!(pretty, "{\n  \"z\": 1,\n  \"a\": [\n    1,\n    2,\n    {}\n  ],\n  \"s\": \"x, y: {z}\",\n  \"e\": []\n}");
        assert!(pretty_json("not json {").is_none());
        assert!(pretty_json("{\"a\": 1").is_none());
    }
}
