//! A reader that quits early (`| head`) closes the pipe.
//!
//! Rust ignores `SIGPIPE`, so the failed write comes back as an error and
//! `println!` panics on it. That is not a failure: the output simply has
//! nowhere left to go. With `--all`, it also means there is no reason to fetch
//! another page (decision 10).

mod common;

use common::{QUERY_PATH, SENTINEL_TOKEN, SITE, base_url, cli, row, rows};
use httptest::matchers::{json_decoded, request};
use httptest::responders::status_code;
use httptest::{Expectation, Server, all_of};
use serde_json::Value;
use std::io::Read;
use std::process::Stdio;

/// Runs the binary with its stdout already closed, and checks that it exits 0
/// without a panic.
fn closed_stdout(server: &Server, args: &[&str]) {
    let mut child = cli()
        .args(args)
        .args(["--base-url", &base_url(server)])
        .env("GSC_ACCESS_TOKEN", SENTINEL_TOKEN)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary runs");
    drop(child.stdout.take());
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr is piped")
        .read_to_string(&mut stderr)
        .expect("stderr reads");
    let status = child.wait().expect("the child exits");

    assert!(
        !stderr.contains("panicked"),
        "panicked on a closed pipe:\n{stderr}"
    );
    assert_eq!(status.code(), Some(0), "{stderr}");
}

#[test]
fn a_closed_stdout_stops_the_pages_and_is_not_a_panic() {
    let server = Server::run();
    // Full pages of 25,000 rows: without the stop, the pages never end.
    let page: Vec<Value> = (0..25_000)
        .map(|i| row(&[&format!("/p{i}")], 1, 1))
        .collect();
    let body = rows(&page);
    let at = |start: u64| {
        all_of![
            request::method_path("POST", QUERY_PATH),
            request::body(json_decoded(move |b: &Value| b["startRow"] == start)),
        ]
    };
    server.expect(
        Expectation::matching(at(0))
            .times(1)
            .respond_with(status_code(200).body(body.clone())),
    );
    // Allowed, not required: the second page may already be on its way.
    server.expect(
        Expectation::matching(at(25_000))
            .times(..=1)
            .respond_with(status_code(200).body(body)),
    );

    closed_stdout(
        &server,
        &[
            "query",
            "-s",
            SITE,
            "-d",
            "page",
            "--start-date",
            "2026-10-01",
            "--end-date",
            "2026-10-28",
            "--all",
            "--format",
            "jsonl",
        ],
    );
    // `server` checks on drop that no third page was asked for.
}

/// The CSV header is the first write when the rows come all at once: after a
/// sort, and in `compare`.
#[test]
fn a_closed_stdout_before_the_csv_header_is_not_an_error() {
    let server = Server::run();
    server.expect(
        Expectation::matching(request::method_path("POST", QUERY_PATH))
            .times(5)
            .respond_with(status_code(200).body(rows(&[row(&["/a"], 1, 10)]))),
    );
    let period = ["--start-date", "2026-10-01", "--end-date", "2026-10-28"];
    let csv = ["--format", "csv"];

    let mut query = vec!["query", "-s", SITE, "-d", "page", "--sort", "clicks"];
    query.extend(period.iter().chain(&csv));
    closed_stdout(&server, &query);

    let mut compare = vec!["compare", "-s", SITE, "-d", "page"];
    compare.extend(period.iter().chain(&csv));
    closed_stdout(&server, &compare);
}
