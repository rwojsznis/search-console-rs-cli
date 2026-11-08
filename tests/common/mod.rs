//! The shared harness: run the real binary against a mock Google.

// Each test file compiles this module on its own and uses part of it, so the
// rest is dead code in that file.
#![allow(dead_code)]

use httptest::matchers::{all_of, contains, eq, json_decoded, request};
use httptest::{Expectation, ExpectationBuilder, Server};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

/// An access token that must never appear in any output (decision 6).
pub(crate) const SENTINEL_TOKEN: &str = "sentinel-token-7f3a";

pub(crate) const SITE: &str = "sc-domain:example.com";
pub(crate) const SITE_PATH: &str = "/webmasters/v3/sites/sc-domain%3Aexample.com";
pub(crate) const QUERY_PATH: &str =
    "/webmasters/v3/sites/sc-domain%3Aexample.com/searchAnalytics/query";

/// Every variable the CLI reads, removed so that the developer's own
/// credentials and settings cannot leak into a test.
const ENV: [&str; 15] = [
    "GSC_ACCESS_TOKEN",
    "GSC_CREDENTIALS",
    "GSC_BASE_URL",
    "GSC_SITE",
    "GSC_CONFIG_DIR",
    "GSC_CLIENT_SECRET",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "CLOUDSDK_CONFIG",
    "XDG_CONFIG_HOME",
    "BROWSER",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
];

/// An empty home, so that `~/.config/gsc-cli` and gcloud's file are never
/// the developer's.
fn empty_home() -> PathBuf {
    let home = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("empty-home");
    std::fs::create_dir_all(&home).expect("the empty home is created");
    home
}

/// The binary, with every variable of `ENV` removed and `HOME` empty.
///
/// Never `env_clear()`: it removes `LLVM_PROFILE_FILE`, and cargo-llvm-cov
/// then loses the coverage of the spawned binary (decision 14).
pub(crate) fn cli() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_gsc-cli"));
    for var in ENV {
        cmd.env_remove(var);
    }
    cmd.env("HOME", empty_home());
    cmd
}

#[derive(Debug)]
pub(crate) struct Run {
    pub(crate) code: i32,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

impl Run {
    /// stdout as one JSON value.
    pub(crate) fn json(&self) -> Value {
        serde_json::from_str(&self.stdout)
            .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {:?}", self.stdout))
    }

    /// Every stdout line as JSON.
    pub(crate) fn json_lines(&self) -> Vec<Value> {
        self.stdout
            .lines()
            .map(|line| {
                serde_json::from_str(line)
                    .unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {line:?}"))
            })
            .collect()
    }

    /// Every stderr line as JSON. Decision 5: errors, warnings and the
    /// `--verbose` lines are all one JSON value per line.
    pub(crate) fn stderr_lines(&self) -> Vec<Value> {
        self.stderr
            .lines()
            .map(|line| {
                serde_json::from_str(line)
                    .unwrap_or_else(|e| panic!("stderr line is not JSON ({e}): {line:?}"))
            })
            .collect()
    }

    /// The `error` object of the one error line, which comes last.
    pub(crate) fn error(&self) -> Value {
        let lines = self.stderr_lines();
        let last = lines.last().expect("an error line on stderr");
        last.get("error")
            .unwrap_or_else(|| panic!("the last stderr line is not an error: {last}"))
            .clone()
    }

    /// The kinds of the warning lines, in order.
    pub(crate) fn warnings(&self) -> Vec<String> {
        self.stderr_lines()
            .iter()
            .filter_map(|line| line.pointer("/warning/kind"))
            .map(|kind| kind.as_str().unwrap().to_owned())
            .collect()
    }
}

pub(crate) fn run(args: &[&str], env: &[(&str, &str)]) -> Run {
    let mut cmd = cli();
    cmd.args(args);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("the binary runs");
    Run {
        code: out.status.code().expect("an exit code, not a signal"),
        stdout: String::from_utf8(out.stdout).expect("stdout is UTF-8"),
        stderr: String::from_utf8(out.stderr).expect("stderr is UTF-8"),
    }
}

/// Runs against `server` with the sentinel access token.
pub(crate) fn run_api(server: &Server, args: &[&str]) -> Run {
    let base = base_url(server);
    let mut all = args.to_vec();
    all.extend(["--base-url", &base]);
    run(&all, &[("GSC_ACCESS_TOKEN", SENTINEL_TOKEN)])
}

/// Asserts that no secret is in either stream.
pub(crate) fn assert_no_secret(run: &Run, secrets: &[&str]) {
    for secret in secrets.iter().chain(&[SENTINEL_TOKEN]) {
        assert!(
            !run.stdout.contains(secret),
            "stdout has {secret}: {}",
            run.stdout
        );
        assert!(
            !run.stderr.contains(secret),
            "stderr has {secret}: {}",
            run.stderr
        );
    }
}

/// The `--base-url` for a mock server.
pub(crate) fn base_url(server: &Server) -> String {
    server.url_str("/")
}

/// Expects one GET on `path` with exactly `query` (encoded, as on the wire;
/// `""` for none) and the sentinel bearer token.
pub(crate) fn expect_get(path: &str, query: &str) -> ExpectationBuilder {
    Expectation::matching(all_of![
        request::method_path("GET", path.to_owned()),
        request::query(eq(query.to_owned())),
        request::headers(contains((
            "authorization",
            format!("Bearer {SENTINEL_TOKEN}")
        ))),
        request::headers(contains(("accept", "application/json"))),
    ])
}

/// Expects one POST on `path` with exactly this JSON body and the sentinel
/// bearer token.
pub(crate) fn expect_post(path: &str, body: Value) -> ExpectationBuilder {
    Expectation::matching(all_of![
        request::method_path("POST", path.to_owned()),
        request::headers(contains((
            "authorization",
            format!("Bearer {SENTINEL_TOKEN}")
        ))),
        request::headers(contains(("content-type", "application/json"))),
        request::body(json_decoded(eq(body))),
    ])
}

/// Expects one Search Analytics query whose body has `key` equal to `value`
/// (for the probe, whose dates depend on today).
pub(crate) fn expect_query_with(key: &'static str, value: Value) -> ExpectationBuilder {
    Expectation::matching(all_of![
        request::method_path("POST", QUERY_PATH),
        request::body(json_decoded(
            move |body: &Value| body.get(key) == Some(&value)
        )),
    ])
}

/// A Search Analytics answer with these rows.
pub(crate) fn rows(rows: &[Value]) -> String {
    serde_json::json!({"rows": rows, "responseAggregationType": "byPage"}).to_string()
}

/// One API row.
pub(crate) fn row(keys: &[&str], clicks: u64, impressions: u64) -> Value {
    #[allow(clippy::cast_precision_loss)]
    let ctr = if impressions == 0 {
        0.0
    } else {
        clicks as f64 / impressions as f64
    };
    serde_json::json!({"keys": keys, "clicks": clicks, "impressions": impressions, "ctr": ctr, "position": 3.5})
}

/// Expects the probe for the latest day with data (decision 9): daily rows
/// and no paging. Its dates depend on today, so they are not matched.
pub(crate) fn expect_probe() -> ExpectationBuilder {
    Expectation::matching(all_of![
        request::method_path("POST", QUERY_PATH),
        request::body(json_decoded(|body: &Value| {
            body["dimensions"] == serde_json::json!(["date"]) && body.get("startRow").is_none()
        })),
    ])
}

/// A probe answer whose latest day is `latest`.
pub(crate) fn probe_answer(latest: &str) -> String {
    rows(&[row(&["2020-01-01"], 1, 1), row(&[latest], 1, 1)])
}
