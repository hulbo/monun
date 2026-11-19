//! Sends a request of an API tab and reads its answer, off the interface's thread (`ureq`, with
//! rustls checking certificates by Windows' own store, so a company's root certificate and its
//! inspecting proxy are trusted as the browser trusts them; a proxy from `HTTPS_PROXY`).

use std::time::Duration;

/// A request ready to go: its variables read.
#[derive(Clone, Debug)]
pub struct Outgoing {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// What goes as the body: its files are read on the way.
    pub body: super::body::Payload,
    /// Certificates go unchecked (a development server's own).
    pub insecure: bool,
}

/// The answer to a request.
#[derive(Clone, Debug, Default)]
pub struct Answer {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// The body was longer than [`BODY_MAX`]: what shows is its start.
    pub cut: bool,
}

/// The longest answer read.
pub const BODY_MAX: u64 = 50 * 1024 * 1024;

/// How long a request may take, all told.
pub const TIMEOUT: Duration = Duration::from_secs(60);

/// Sends `out` and waits for its answer.
#[cfg(feature = "api")]
pub fn send(out: &Outgoing) -> Result<Answer, String> {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    let tls = TlsConfig::builder()
        .provider(TlsProvider::Rustls)
        .root_certs(if out.insecure { RootCerts::WebPki } else { RootCerts::PlatformVerifier })
        .disable_verification(out.insecure)
        .build();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(tls)
        .http_status_as_error(false)
        .timeout_global(Some(TIMEOUT))
        .user_agent(concat!("Monun/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let url = out.url.as_str();
    enum Builder {
        Bare(ureq::RequestBuilder<ureq::typestate::WithoutBody>),
        Body(ureq::RequestBuilder<ureq::typestate::WithBody>),
    }
    let mut b = match out.method.as_str() {
        "POST" => Builder::Body(agent.post(url)),
        "PUT" => Builder::Body(agent.put(url)),
        "PATCH" => Builder::Body(agent.patch(url)),
        "DELETE" => Builder::Bare(agent.delete(url)),
        "HEAD" => Builder::Bare(agent.head(url)),
        "OPTIONS" => Builder::Bare(agent.options(url)),
        _ => Builder::Bare(agent.get(url)),
    };
    for (k, v) in &out.headers {
        b = match b {
            Builder::Bare(r) => Builder::Bare(r.header(k.as_str(), v.as_str())),
            Builder::Body(r) => Builder::Body(r.header(k.as_str(), v.as_str())),
        };
    }
    let body = super::body::bytes(&out.body)?;
    let answer = match b {
        Builder::Body(r) => r.send(body.as_slice()),
        Builder::Bare(r) if body.is_empty() => r.call(),
        Builder::Bare(r) => r.force_send_body().send(body.as_slice()),
    };
    let mut answer = answer.map_err(|e| describe(&e))?;
    let status = answer.status();
    let headers = answer.headers().iter().map(|(k, v)| (k.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())).collect();
    let (body, cut) = match answer.body_mut().with_config().limit(BODY_MAX).read_to_vec() {
        Ok(b) => (b, false),
        Err(ureq::Error::BodyExceedsLimit(_)) => (Vec::new(), true),
        Err(e) => return Err(describe(&e)),
    };
    Ok(Answer { status: status.as_u16(), reason: status.canonical_reason().unwrap_or_default().to_string(), headers, body, cut })
}

/// Without the `api` feature (the web app) nothing is sent.
#[cfg(not(feature = "api"))]
pub fn send(_out: &Outgoing) -> Result<Answer, String> {
    Err("Sending requests needs the desktop app.".into())
}

/// Why a request failed: a sentence the tab translates, then (after a line break) what the
/// library said, as it said it.
#[cfg(feature = "api")]
fn describe(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Timeout(_) => "The server didn't answer in time.".into(),
        ureq::Error::HostNotFound => "The server's name wasn't found (DNS).".into(),
        ureq::Error::ConnectionFailed => "Couldn't connect to the server.".into(),
        ureq::Error::BadUri(u) => format!(
            "The address isn't valid.
{u}"
        ),
        ureq::Error::Tls(t) => format!(
            "The secure connection failed.
{t}"
        ),
        ureq::Error::Rustls(t) => format!(
            "The secure connection failed.
{t}"
        ),
        ureq::Error::Io(io) => format!(
            "Connection error.
{io}"
        ),
        other => format!(
            "The request failed.
{other}"
        ),
    }
}
