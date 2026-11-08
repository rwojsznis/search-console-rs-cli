//! The one HTTP client (decision 2): a timeout, retries on 429 and 5xx
//! (decision 12), and the `--verbose` lines (decision 7).

use crate::errln;
use crate::error::{Error, describe};
use crate::output::warn;
use serde_json::{Value, json};
use std::fmt;
use std::time::{Duration, Instant};
use url::Url;

/// The statuses Google documents as worth a retry with backoff.
const RETRY_STATUSES: [u16; 5] = [429, 500, 502, 503, 504];
/// The first backoff; it doubles on each retry.
const RETRY_BASE: Duration = Duration::from_millis(500);
/// The longest `Retry-After` we wait for.
const RETRY_AFTER_MAX: u64 = 60;
/// ureq's default body limit is 10 MiB. A page of 25,000 rows with long URLs
/// and queries can pass that.
const BODY_MAX: u64 = 512 * 1024 * 1024;

/// The two methods the CLI sends. There is no other (decision 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

impl Method {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
        }
    }
}

/// A request body.
#[derive(Debug, Clone, Copy)]
pub enum Body<'a> {
    None,
    Json(&'a Value),
    /// `application/x-www-form-urlencoded`, for the token endpoint.
    Form(&'a [(&'a str, &'a str)]),
}

/// Turns a non-2xx status and its body into the error for that server:
/// `Error::api` or `Error::token`.
pub type OnStatus = fn(u16, &[u8]) -> Error;

pub struct Http {
    agent: ureq::Agent,
    timeout: Duration,
    retries: u32,
    verbose: bool,
}

impl fmt::Debug for Http {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Http")
            .field("timeout", &self.timeout)
            .field("retries", &self.retries)
            .field("verbose", &self.verbose)
            .finish_non_exhaustive()
    }
}

/// One answer: the status, `Retry-After` in seconds, and the body.
struct Answer {
    status: u16,
    retry_after: Option<u64>,
    body: Vec<u8>,
}

impl Http {
    #[must_use]
    pub fn new(timeout: Duration, retries: u32, verbose: bool, user_agent: &str) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .user_agent(user_agent)
            .build();
        Http {
            agent: ureq::Agent::new_with_config(config),
            timeout,
            retries,
            verbose,
        }
    }

    /// Sends one request and returns the JSON body of its 2xx answer. A 429
    /// or 5xx is retried up to `--retries` times, after `Retry-After` or an
    /// exponential backoff.
    ///
    /// # Errors
    ///
    /// `on_status` for a non-2xx answer; `transport`, `timeout` or `decode`
    /// otherwise. The error carries the request.
    pub fn send(
        &self,
        method: Method,
        url: &Url,
        bearer: Option<&str>,
        body: Body,
        on_status: OnStatus,
    ) -> Result<Value, Error> {
        let mut attempt = 0;
        loop {
            let answer = self
                .logged(method, url, bearer, body)
                .map_err(|e| e.at(method.as_str(), url))?;
            if (200..300).contains(&answer.status) {
                return serde_json::from_slice(&answer.body).map_err(|e| {
                    Error::decode(format!("the response is not JSON: {e}")).at(method.as_str(), url)
                });
            }
            if attempt < self.retries && RETRY_STATUSES.contains(&answer.status) {
                let delay = answer
                    .retry_after
                    .map_or(RETRY_BASE * 2_u32.pow(attempt), |s| {
                        Duration::from_secs(s.min(RETRY_AFTER_MAX))
                    });
                attempt += 1;
                warn(
                    "retry",
                    &format!(
                        "{} answered {}; retry {attempt} of {} in {:.1} s",
                        url,
                        answer.status,
                        self.retries,
                        delay.as_secs_f64()
                    ),
                );
                std::thread::sleep(delay);
                continue;
            }
            return Err(on_status(answer.status, &answer.body).at(method.as_str(), url));
        }
    }

    /// One attempt, between its two `--verbose` lines.
    fn logged(
        &self,
        method: Method,
        url: &Url,
        bearer: Option<&str>,
        body: Body,
    ) -> Result<Answer, Error> {
        if self.verbose {
            errln!(
                "{}",
                json!({"request": {"method": method.as_str(), "url": url.as_str()}})
            );
        }
        let started = Instant::now();
        let result = self.attempt(method, url, bearer, body);
        if self.verbose {
            let status = match &result {
                Ok(answer) => json!(answer.status),
                Err(error) => json!(error.kind().as_str()),
            };
            let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            errln!("{}", json!({"response": {"status": status, "ms": ms}}));
        }
        result
    }

    fn attempt(
        &self,
        method: Method,
        url: &Url,
        bearer: Option<&str>,
        body: Body,
    ) -> Result<Answer, Error> {
        let authorization = bearer.map(|token| format!("Bearer {token}"));
        let sent = match method {
            Method::Get => {
                let mut request = self
                    .agent
                    .get(url.as_str())
                    .header("Accept", "application/json");
                if let Some(value) = &authorization {
                    request = request.header("Authorization", value);
                }
                request.call()
            }
            Method::Post => {
                let mut request = self
                    .agent
                    .post(url.as_str())
                    .header("Accept", "application/json");
                if let Some(value) = &authorization {
                    request = request.header("Authorization", value);
                }
                match body {
                    Body::None => request.send_empty(),
                    Body::Json(value) => request
                        .content_type("application/json")
                        .send(value.to_string()),
                    Body::Form(pairs) => request.send_form(pairs.iter().copied()),
                }
            }
        };
        let mut response = sent.map_err(|e| self.failure(&e))?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse().ok());
        let body = response
            .body_mut()
            .with_config()
            .limit(BODY_MAX)
            .read_to_vec()
            .map_err(|e| self.failure(&e))?;
        Ok(Answer {
            status,
            retry_after,
            body,
        })
    }

    /// ureq's messages name the URL, never the headers, so no token.
    fn failure(&self, error: &ureq::Error) -> Error {
        match error {
            ureq::Error::Timeout(_) => Error::timeout(self.timeout.as_secs()),
            other => Error::transport(describe(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Kind;
    use httptest::matchers::{all_of, contains, request, url_decoded};
    use httptest::responders::status_code;
    use httptest::{Expectation, Server};

    fn http(retries: u32) -> Http {
        Http::new(Duration::from_secs(5), retries, false, "gsc-cli/test")
    }

    fn url(server: &Server, path: &str) -> Url {
        Url::parse(&server.url_str(path)).unwrap()
    }

    #[test]
    fn a_form_is_url_encoded() {
        let server = Server::run();
        server.expect(
            Expectation::matching(all_of![
                request::method_path("POST", "/token"),
                request::headers(contains((
                    "content-type",
                    "application/x-www-form-urlencoded"
                ))),
                request::body(url_decoded(contains(("a", "1 2")))),
            ])
            .respond_with(status_code(200).body(r#"{"ok":true}"#)),
        );
        let value = http(0)
            .send(
                Method::Post,
                &url(&server, "/token"),
                None,
                Body::Form(&[("a", "1 2")]),
                Error::token,
            )
            .unwrap();
        assert_eq!(value, json!({"ok": true}));
    }

    #[test]
    fn an_empty_post_and_a_retry_after_are_honoured() {
        let server = Server::run();
        server.expect(
            Expectation::matching(request::method_path("POST", "/x"))
                .times(2)
                .respond_with(httptest::cycle![
                    status_code(429).insert_header("Retry-After", "0"),
                    status_code(200).body("{}"),
                ]),
        );
        let started = Instant::now();
        let value = http(1)
            .send(
                Method::Post,
                &url(&server, "/x"),
                None,
                Body::None,
                Error::api,
            )
            .unwrap();
        assert_eq!(value, json!({}));
        assert!(
            started.elapsed() < RETRY_BASE,
            "Retry-After: 0 means no wait"
        );
    }

    #[test]
    fn retries_run_out() {
        let server = Server::run();
        server.expect(
            Expectation::matching(request::method_path("GET", "/x"))
                .times(1)
                .respond_with(status_code(503)),
        );
        let error = http(0)
            .send(
                Method::Get,
                &url(&server, "/x"),
                None,
                Body::None,
                Error::api,
            )
            .unwrap_err();
        assert_eq!(error.kind(), Kind::Api);
        assert_eq!(error.to_json()["error"]["status"], 503);
    }

    #[test]
    fn debug_shows_the_settings_only() {
        let debug = format!("{:?}", http(2));
        assert!(debug.contains("retries: 2"), "{debug}");
    }
}
