//! Every error path: one JSON line on stderr, nothing on stdout, the exit code
//! of decision 5, and never a secret (decision 6).

mod common;

use common::{
    Run, SENTINEL_TOKEN, SITE_PATH, assert_no_secret, base_url, expect_get, run, run_api,
};
use httptest::Server;
use httptest::responders::{delay_and_then, status_code};
use serde_json::json;
use std::time::Duration;

fn sites_get(server: &Server, extra: &[&str]) -> Run {
    let mut args = vec!["sites", "get", "-s", common::SITE];
    args.extend_from_slice(extra);
    run_api(server, &args)
}

/// The checks every error shares.
fn assert_error(out: &Run, code: i32, kind: &str) -> serde_json::Value {
    assert_eq!(out.code, code, "{out:?}");
    assert_eq!(out.stdout, "", "an error prints nothing on stdout");
    assert_no_secret(out, &[]);
    let error = out.error();
    assert_eq!(error["kind"], kind, "{error}");
    assert!(error["message"].is_string(), "{error}");
    error
}

fn google_error(status: u16, message: &str, reason: &str) -> String {
    json!({"error": {"code": status, "message": message, "status": reason}}).to_string()
}

#[test]
fn a_permission_error_has_the_status_reason_hint_and_request() {
    let server = Server::run();
    server.expect(
        expect_get(SITE_PATH, "").respond_with(status_code(403).body(google_error(
            403,
            "User does not have sufficient permission for site 'sc-domain:example.com'.",
            "PERMISSION_DENIED",
        ))),
    );

    let out = sites_get(&server, &[]);

    let error = assert_error(&out, 1, "api");
    assert_eq!(out.stderr.lines().count(), 1);
    assert_eq!(error["status"], 403);
    assert_eq!(error["reason"], "PERMISSION_DENIED");
    assert!(
        error["hint"].as_str().unwrap().contains("sites list"),
        "{error}"
    );
    assert_eq!(
        error["request"],
        json!({"method": "GET", "url": format!("{}{}", base_url(&server).trim_end_matches('/'), SITE_PATH)})
    );
}

#[test]
fn a_401_is_an_auth_error_with_exit_4() {
    let server = Server::run();
    server.expect(
        expect_get(SITE_PATH, "").respond_with(status_code(401).body(google_error(
            401,
            "Request had invalid authentication credentials.",
            "UNAUTHENTICATED",
        ))),
    );
    let error = assert_error(&sites_get(&server, &[]), 4, "auth");
    assert!(
        error["hint"].as_str().unwrap().contains("auth status"),
        "{error}"
    );
}

#[test]
fn a_503_is_retried_with_a_warning() {
    let server = Server::run();
    server.expect(
        expect_get(SITE_PATH, "")
            .times(2)
            .respond_with(httptest::cycle![
                status_code(503),
                status_code(200).body(r#"{"siteUrl":"sc-domain:example.com"}"#),
            ]),
    );
    let out = sites_get(&server, &["--retries", "1"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.warnings(), ["retry"]);
    assert_no_secret(&out, &[]);
}

#[test]
fn retries_run_out_into_the_last_error() {
    let server = Server::run();
    server.expect(expect_get(SITE_PATH, "").times(1).respond_with(
        status_code(429).body(google_error(429, "Quota exceeded", "RESOURCE_EXHAUSTED")),
    ));
    let error = assert_error(&sites_get(&server, &["--retries", "0"]), 1, "api");
    assert!(error["hint"].as_str().unwrap().contains("Quota"), "{error}");
}

#[test]
fn a_2xx_body_that_is_not_json_is_a_decode_error() {
    let server = Server::run();
    server.expect(expect_get(SITE_PATH, "").respond_with(status_code(200).body("<html>")));
    assert_error(&sites_get(&server, &[]), 3, "decode");
}

#[test]
fn no_answer_within_the_timeout_is_a_timeout_error() {
    let server = Server::run();
    server.expect(
        expect_get(SITE_PATH, "")
            .respond_with(delay_and_then(Duration::from_secs(3), status_code(200))),
    );
    let error = assert_error(&sites_get(&server, &["--timeout", "1"]), 3, "timeout");
    assert_eq!(error["message"], "no response within 1 s");
}

#[test]
fn a_refused_connection_is_a_transport_error() {
    // Nothing listens on port 1.
    let out = run(
        &["sites", "list", "--base-url", "http://127.0.0.1:1/"],
        &[("GSC_ACCESS_TOKEN", SENTINEL_TOKEN)],
    );
    let error = assert_error(&out, 3, "transport");
    assert_eq!(
        error["request"]["url"],
        "http://127.0.0.1:1/webmasters/v3/sites"
    );
}

/// The arguments and the environment of one run.
type Case<'a> = (&'a [&'a str], &'a [(&'a str, &'a str)]);

#[test]
fn configuration_errors_send_nothing() {
    // No expectations: a request would fail the test when `server` drops.
    let server = Server::run();
    let base = base_url(&server);
    let cases: [Case; 4] = [
        (
            &["sites", "list", "--base-url", "ftp://x"],
            &[("GSC_ACCESS_TOKEN", SENTINEL_TOKEN)],
        ),
        (
            &["sites", "list", "--base-url", &base],
            &[("GSC_ACCESS_TOKEN", "two words")],
        ),
        (
            &["sites", "list", "--base-url", &base, "--format", "csv"],
            &[("GSC_ACCESS_TOKEN", SENTINEL_TOKEN)],
        ),
        (&["commands", "--format", "csv"], &[]),
    ];
    for (args, env) in cases {
        let out = run(args, env);
        assert_error(&out, 2, "config");
        assert!(!out.stderr.contains("two words"), "{out:?}");
    }
}

#[test]
fn no_credentials_is_an_auth_error_that_says_where_to_look() {
    let server = Server::run();
    let out = run(&["sites", "list", "--base-url", &base_url(&server)], &[]);
    let error = assert_error(&out, 4, "auth");
    assert!(
        error["hint"].as_str().unwrap().contains("GSC_CREDENTIALS"),
        "{error}"
    );
    assert_eq!(error["request"]["method"], "GET");
}

#[test]
fn verbose_logs_each_request_and_response_as_json() {
    let server = Server::run();
    server.expect(expect_get(SITE_PATH, "").respond_with(status_code(200).body("{}")));
    let out = sites_get(&server, &["--verbose"]);
    assert_eq!(out.code, 0, "{out:?}");
    let lines = out.stderr_lines();
    assert_eq!(lines.len(), 2, "{out:?}");
    assert_eq!(lines[0]["request"]["method"], "GET");
    assert_eq!(lines[1]["response"]["status"], 200);
    assert!(lines[1]["response"]["ms"].is_u64());
    assert_no_secret(&out, &[]);

    let refused = run(
        &[
            "sites",
            "list",
            "--verbose",
            "--base-url",
            "http://127.0.0.1:1/",
        ],
        &[("GSC_ACCESS_TOKEN", SENTINEL_TOKEN)],
    );
    assert_eq!(refused.stderr_lines()[1]["response"]["status"], "transport");
}
