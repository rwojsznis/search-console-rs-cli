//! Errors, their exit codes, and the one JSON line that reports them
//! (decision 5).
//!
//! An agent branches on `kind` and the exit code, so the mapping from a
//! failure to a kind is the contract: `api` means Search Console answered, so
//! a retry gets the same answer; `auth` means a person must fix the
//! credentials; `transport`, `timeout` and `decode` mean no usable answer came
//! back; `config` means nothing was sent.

use crate::cli::exit::ExitStatus;
use serde_json::{Map, Value, json};
use url::Url;

/// The longest non-JSON error body kept in the error line. An HTML error page
/// from a proxy is noise after the first few lines.
const BODY_LIMIT: usize = 4096;

/// The advice every credentials problem ends with.
const AUTH_STATUS: &str = "`gsc-cli auth status` shows which credentials are used; \
                           `gsc-cli auth login` makes new ones.";

/// What went wrong, and the request it went wrong for.
///
/// Boxed, because every fallible function returns it, and the unboxed size (a
/// `Url` and a JSON body) would make each `Result` that large.
#[derive(Debug)]
pub struct Error(Box<Inner>);

#[derive(Debug)]
struct Inner {
    kind: Kind,
    message: String,
    /// What to do about it, for an agent to act on.
    hint: Option<String>,
    /// The HTTP status, when a server answered.
    status: Option<u16>,
    /// Google's own name for the error: `PERMISSION_DENIED`, `invalid_grant`.
    reason: Option<String>,
    /// The parsed body when it is JSON, otherwise the text, cut to 4 KiB.
    body: Option<Value>,
    /// The first request this error belongs to.
    request: Option<(&'static str, Url)>,
}

/// The `kind` in the JSON line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Search Console answered with a non-2xx status.
    Api,
    /// No credentials, unreadable credentials, or Google refused them.
    Auth,
    /// The command line or the configuration. Nothing was sent.
    Config,
    /// Connect, TLS, or any other failure before a response came back.
    Transport,
    /// No response within `--timeout`.
    Timeout,
    /// A 2xx response whose body is not the JSON we expect.
    Decode,
}

impl Kind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Api => "api",
            Kind::Auth => "auth",
            Kind::Config => "config",
            Kind::Transport => "transport",
            Kind::Timeout => "timeout",
            Kind::Decode => "decode",
        }
    }

    #[must_use]
    pub fn exit_status(self) -> ExitStatus {
        match self {
            Kind::Api => ExitStatus::Api,
            Kind::Auth => ExitStatus::Auth,
            Kind::Config => ExitStatus::Usage,
            Kind::Transport | Kind::Timeout | Kind::Decode => ExitStatus::Transport,
        }
    }
}

impl Error {
    #[must_use]
    pub fn new(kind: Kind, message: impl Into<String>) -> Self {
        Error(Box::new(Inner {
            kind,
            message: message.into(),
            hint: None,
            status: None,
            reason: None,
            body: None,
            request: None,
        }))
    }

    #[must_use]
    pub fn config(message: impl Into<String>) -> Self {
        Error::new(Kind::Config, message)
    }

    #[must_use]
    pub fn auth(message: impl Into<String>) -> Self {
        Error::new(Kind::Auth, message).with_hint(AUTH_STATUS)
    }

    #[must_use]
    pub fn transport(message: impl Into<String>) -> Self {
        Error::new(Kind::Transport, message)
    }

    #[must_use]
    pub fn decode(message: impl Into<String>) -> Self {
        Error::new(Kind::Decode, message)
    }

    #[must_use]
    pub fn timeout(seconds: u64) -> Self {
        Error::new(Kind::Timeout, format!("no response within {seconds} s"))
    }

    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.0.hint = Some(hint.into());
        self
    }

    /// A non-2xx answer from the Search Console API. Google's error body is
    /// `{"error": {"code", "message", "status", "errors": [...]}}`. A 401 is
    /// an `auth` error: the token was refused, and only new credentials help.
    #[must_use]
    pub fn api(status: u16, body: &[u8]) -> Self {
        let body = body_value(body);
        let error = body.get("error").filter(|e| e.is_object());
        let text = |pointer: &str| {
            error
                .and_then(|e| e.pointer(pointer))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let message = text("/message");
        let reason = text("/status").or_else(|| text("/errors/0/reason"));
        let kind = if status == 401 { Kind::Auth } else { Kind::Api };
        let summary = match &message {
            Some(message) => format!("Search Console returned {status}: {message}"),
            None => format!("Search Console returned {status}"),
        };
        Error(Box::new(Inner {
            kind,
            hint: api_hint(status, reason.as_deref(), message.as_deref().unwrap_or("")),
            message: summary,
            status: Some(status),
            reason,
            body: Some(body),
            request: None,
        }))
    }

    /// A non-2xx answer from the OAuth token endpoint:
    /// `{"error": "invalid_grant", "error_description": "..."}`. Always an
    /// `auth` error. The body never holds the secret that was sent.
    #[must_use]
    pub fn token(status: u16, body: &[u8]) -> Self {
        let body = body_value(body);
        let reason = body.get("error").and_then(Value::as_str).map(str::to_owned);
        let mut message = format!("the token endpoint returned {status}");
        for part in [
            reason.as_deref(),
            body.get("error_description").and_then(Value::as_str),
        ]
        .into_iter()
        .flatten()
        {
            message.push_str(": ");
            message.push_str(part);
        }
        Error(Box::new(Inner {
            kind: Kind::Auth,
            hint: Some(token_hint(reason.as_deref())),
            message,
            status: Some(status),
            reason,
            body: Some(body),
            request: None,
        }))
    }

    /// Records the request this error belongs to. The first one wins: a token
    /// error keeps the token endpoint, not the API request that needed it.
    #[must_use]
    pub fn at(mut self, method: &'static str, url: &Url) -> Self {
        if self.0.request.is_none() {
            self.0.request = Some((method, url.clone()));
        }
        self
    }

    #[must_use]
    pub fn kind(&self) -> Kind {
        self.0.kind
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.0.message
    }

    #[must_use]
    pub fn hint(&self) -> Option<&str> {
        self.0.hint.as_deref()
    }

    #[must_use]
    pub fn exit_status(&self) -> ExitStatus {
        self.0.kind.exit_status()
    }

    /// `{"error": {...}}`, in the key order of decision 5.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let inner = &self.0;
        let mut error = Map::new();
        error.insert("kind".into(), inner.kind.as_str().into());
        error.insert("message".into(), inner.message.clone().into());
        if let Some(hint) = &inner.hint {
            error.insert("hint".into(), hint.clone().into());
        }
        if let Some(status) = inner.status {
            error.insert("status".into(), status.into());
        }
        if let Some(reason) = &inner.reason {
            error.insert("reason".into(), reason.clone().into());
        }
        if let Some(body) = &inner.body {
            error.insert("body".into(), body.clone());
        }
        if let Some((method, url)) = &inner.request {
            error.insert(
                "request".into(),
                json!({"method": method, "url": url.as_str()}),
            );
        }
        json!({ "error": error })
    }
}

/// What to do about the API errors an agent can act on. Matched on Google's
/// message text where Google gives no machine-readable reason.
fn api_hint(status: u16, reason: Option<&str>, message: &str) -> Option<String> {
    let message = message.to_ascii_lowercase();
    let hint = if status == 401 {
        format!(
            "Google refused the access token: it is expired, revoked, or not a Google OAuth \
             token. {AUTH_STATUS}"
        )
    } else if message.contains("insufficient authentication scopes")
        || reason == Some("ACCESS_TOKEN_SCOPE_INSUFFICIENT")
    {
        format!(
            "The token lacks the {} scope. Run `gsc-cli auth login`, or log in to gcloud \
             again with that scope in --scopes.",
            crate::auth::SCOPE
        )
    } else if message.contains("sufficient permission for site") {
        "Pass --site exactly as `gsc-cli sites list` prints it: sc-domain:example.com for a \
         domain property, https://example.com/ (with the trailing slash) for a URL-prefix \
         property. A service account must be added as a user of the property in Search \
         Console."
            .to_owned()
    } else if message.contains("has not been used in project")
        || message.contains("it is disabled")
        || reason == Some("SERVICE_DISABLED")
    {
        "Enable the Google Search Console API in the Google Cloud project of these \
         credentials; the message has the link."
            .to_owned()
    } else if status == 429 || reason == Some("RESOURCE_EXHAUSTED") {
        "Quota exceeded. Wait a minute and retry, or ask for less: fewer dimensions, a \
         shorter date range, fewer URL inspections (2,000 a day per property)."
            .to_owned()
    } else {
        return None;
    };
    Some(hint)
}

/// What to do about an OAuth error code (RFC 6749, section 5.2).
fn token_hint(reason: Option<&str>) -> String {
    match reason {
        Some("invalid_grant") => format!(
            "The refresh token is expired or revoked, the service account key was deleted, or \
             the clock is wrong. {AUTH_STATUS}"
        ),
        Some("invalid_client" | "unauthorized_client" | "deleted_client") => format!(
            "Google does not know this OAuth client, or it was deleted: check client_id and \
             client_secret. {AUTH_STATUS}"
        ),
        _ => AUTH_STATUS.to_owned(),
    }
}

fn body_value(body: &[u8]) -> Value {
    if let Ok(value) = serde_json::from_slice(body) {
        return value;
    }
    let text = String::from_utf8_lossy(body);
    let mut end = text.len().min(BODY_LIMIT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Value::String(text[..end].to_owned())
}

/// An error and its sources, joined with `": "`. A source whose text the
/// message already holds is skipped: many errors repeat their source in their
/// own `Display`.
#[must_use]
pub fn describe(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !message.contains(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = cause.source();
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url() -> Url {
        Url::parse("https://searchconsole.googleapis.com/webmasters/v3/sites").unwrap()
    }

    fn google(status: u16, message: &str, reason: &str) -> Error {
        let body = json!({"error": {"code": status, "message": message, "status": reason}});
        Error::api(status, body.to_string().as_bytes())
    }

    #[test]
    fn each_kind_has_the_exit_code_of_decision_5() {
        let cases = [
            (Kind::Api, "api", ExitStatus::Api),
            (Kind::Auth, "auth", ExitStatus::Auth),
            (Kind::Config, "config", ExitStatus::Usage),
            (Kind::Transport, "transport", ExitStatus::Transport),
            (Kind::Timeout, "timeout", ExitStatus::Transport),
            (Kind::Decode, "decode", ExitStatus::Transport),
        ];
        for (kind, name, status) in cases {
            assert_eq!(kind.as_str(), name);
            assert_eq!(Error::new(kind, "x").exit_status(), status, "{name}");
        }
    }

    #[test]
    fn an_api_error_line_has_status_reason_body_and_request() {
        let error = google(400, "Invalid argument", "INVALID_ARGUMENT").at("POST", &url());
        assert_eq!(
            error.to_json().to_string(),
            r#"{"error":{"kind":"api","message":"Search Console returned 400: Invalid argument","status":400,"reason":"INVALID_ARGUMENT","body":{"error":{"code":400,"message":"Invalid argument","status":"INVALID_ARGUMENT"}},"request":{"method":"POST","url":"https://searchconsole.googleapis.com/webmasters/v3/sites"}}}"#
        );
    }

    #[test]
    fn the_reason_falls_back_to_the_first_errors_entry() {
        let body = json!({"error": {"message": "m", "errors": [{"reason": "forbidden"}]}});
        let error = Error::api(403, body.to_string().as_bytes());
        assert_eq!(error.to_json()["error"]["reason"], "forbidden");
        let bare = Error::api(502, b"<html>bad gateway</html>");
        assert_eq!(bare.message(), "Search Console returned 502");
        assert_eq!(bare.to_json()["error"]["body"], "<html>bad gateway</html>");
    }

    #[test]
    fn the_first_request_wins() {
        let token = Url::parse("https://oauth2.googleapis.com/token").unwrap();
        let error = Error::auth("x").at("POST", &token).at("GET", &url());
        assert_eq!(
            error.to_json()["error"]["request"]["url"],
            "https://oauth2.googleapis.com/token"
        );
    }

    #[test]
    fn a_401_is_an_auth_error() {
        let error = google(401, "Request had invalid authentication credentials.", "x");
        assert_eq!(error.kind(), Kind::Auth);
        assert_eq!(error.exit_status(), ExitStatus::Auth);
        assert!(error.hint().unwrap().contains("auth login"));
    }

    #[test]
    fn known_api_errors_have_a_hint() {
        let cases = [
            (
                google(
                    403,
                    "Request had insufficient authentication scopes.",
                    "PERMISSION_DENIED",
                ),
                "webmasters.readonly",
            ),
            (
                google(403, "x", "ACCESS_TOKEN_SCOPE_INSUFFICIENT"),
                "webmasters.readonly",
            ),
            (
                google(
                    403,
                    "User does not have sufficient permission for site 'x'.",
                    "PERMISSION_DENIED",
                ),
                "sites list",
            ),
            (
                google(
                    403,
                    "Google Search Console API has not been used in project 1 before or it is disabled.",
                    "PERMISSION_DENIED",
                ),
                "Enable",
            ),
            (google(403, "x", "SERVICE_DISABLED"), "Enable"),
            (google(429, "Quota exceeded", "RESOURCE_EXHAUSTED"), "Quota"),
            (google(400, "x", "RESOURCE_EXHAUSTED"), "Quota"),
        ];
        for (error, needle) in cases {
            let hint = error.hint().unwrap();
            assert!(hint.contains(needle), "{needle}: {hint}");
        }
        assert_eq!(
            google(400, "Invalid argument", "INVALID_ARGUMENT").hint(),
            None
        );
        assert_eq!(google(404, "Not found", "NOT_FOUND").hint(), None);
    }

    #[test]
    fn a_token_error_is_an_auth_error_with_the_oauth_code() {
        let error = Error::token(
            400,
            br#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#,
        );
        assert_eq!(error.kind(), Kind::Auth);
        assert_eq!(
            error.message(),
            "the token endpoint returned 400: invalid_grant: Token has been expired or revoked."
        );
        assert_eq!(error.to_json()["error"]["reason"], "invalid_grant");
        assert!(error.hint().unwrap().contains("expired or revoked"));
        let client = Error::token(401, br#"{"error":"invalid_client"}"#);
        assert!(client.hint().unwrap().contains("client_id"));
        let other = Error::token(500, b"oops");
        assert_eq!(other.message(), "the token endpoint returned 500");
        assert_eq!(other.hint(), Some(AUTH_STATUS));
    }

    #[test]
    fn other_kinds_have_no_status_and_request_only_when_one_was_built() {
        assert_eq!(
            Error::config("bad").to_json(),
            json!({"error": {"kind": "config", "message": "bad"}})
        );
        assert_eq!(
            Error::timeout(30).at("GET", &url()).to_json(),
            json!({"error": {
                "kind": "timeout",
                "message": "no response within 30 s",
                "request": {"method": "GET", "url": "https://searchconsole.googleapis.com/webmasters/v3/sites"}
            }})
        );
        assert_eq!(Error::decode("x").kind(), Kind::Decode);
        assert_eq!(Error::transport("x").kind(), Kind::Transport);
    }

    #[test]
    fn a_body_that_is_not_json_is_cut_on_a_char_boundary() {
        // 3-byte characters: 4096 is not a boundary, 4095 is.
        let body = "€".repeat(2000);
        let kept = body_value(body.as_bytes());
        assert_eq!(kept.as_str().unwrap().len(), 4095);
        assert_eq!(body_value(b"short"), json!("short"));
        assert_eq!(body_value(b"\xff"), json!("\u{fffd}"));
    }

    #[test]
    fn describe_joins_the_sources_without_repeating_them() {
        #[derive(Debug)]
        struct Layer(&'static str, Option<Box<Layer>>);
        impl std::fmt::Display for Layer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.0)
            }
        }
        impl std::error::Error for Layer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                self.1.as_deref().map(|l| l as _)
            }
        }
        let error = Layer(
            "io: connect failed",
            Some(Box::new(Layer(
                "connect failed",
                Some(Box::new(Layer("Connection refused", None))),
            ))),
        );
        assert_eq!(describe(&error), "io: connect failed: Connection refused");
    }
}
