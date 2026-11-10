//! `query`: the request body, the latest-day probe, paging, named rows, the
//! three formats, sorting and the warnings (decisions 3, 4, 9, 10).

mod common;

use common::{
    QUERY_PATH, SITE, expect_post, expect_probe, expect_query_with, probe_answer, row, rows,
    run_api,
};
use httptest::Server;
use httptest::responders::status_code;
use serde_json::{Value, json};

fn body(extra: &Value) -> Value {
    let mut body = json!({
        "startDate": "2026-10-01",
        "endDate": "2026-10-28",
        "type": "web",
        "dataState": "final",
        "rowLimit": 1001,
        "startRow": 0,
    });
    for (key, value) in extra.as_object().unwrap() {
        body[key] = value.clone();
    }
    body
}

const DATES: [&str; 4] = ["--start-date", "2026-10-01", "--end-date", "2026-10-28"];

fn query(server: &Server, extra: &[&str]) -> common::Run {
    let mut args = vec!["query", "--site", SITE];
    args.extend(DATES);
    args.extend(extra);
    run_api(server, &args)
}

#[test]
fn rows_get_named_keys_inside_an_envelope() {
    let server = Server::run();
    server.expect(
        expect_post(QUERY_PATH, body(&json!({"dimensions": ["page", "query"]}))).respond_with(
            status_code(200).body(
                json!({
                    "rows": [row(&["https://example.com/a", "shoes"], 3, 40)],
                    "responseAggregationType": "byPage",
                    "metadata": {"first_incomplete_date": "2026-10-27"},
                })
                .to_string(),
            ),
        ),
    );

    let out = query(&server, &["--dimensions", "page,query"]);

    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stderr, "");
    assert_eq!(
        out.json(),
        json!({
            "siteUrl": "sc-domain:example.com",
            "request": {
                "startDate": "2026-10-01",
                "endDate": "2026-10-28",
                "dimensions": ["page", "query"],
                "type": "web",
                "dataState": "final",
            },
            "responseAggregationType": "byPage",
            "metadata": {"first_incomplete_date": "2026-10-27"},
            "rowCount": 1,
            "truncated": false,
            "rows": [{
                "page": "https://example.com/a",
                "query": "shoes",
                "clicks": 3,
                "impressions": 40,
                "ctr": 0.075,
                "position": 3.5,
            }],
        })
    );
}

#[test]
fn filters_and_every_option_go_into_the_body() {
    let server = Server::run();
    server.expect(
        expect_post(
            QUERY_PATH,
            body(&json!({
                "dimensions": ["country", "device", "searchAppearance"],
                "type": "discover",
                "dimensionFilterGroups": [{"groupType": "and", "filters": [
                    {"dimension": "page", "operator": "includingRegex", "expression": "^https://example.com/(a|b)/"},
                    {"dimension": "query", "operator": "notContains", "expression": "brand name"},
                ]}],
                "aggregationType": "byProperty",
                "dataState": "all",
                "rowLimit": 6,
            })),
        )
        .respond_with(status_code(200).body("{}")),
    );

    let out = query(
        &server,
        &[
            "-d",
            "country,DEVICE,searchappearance",
            "-f",
            "page:includingRegex:^https://example.com/(a|b)/",
            "--filter",
            "query:notcontains:brand name",
            "--type",
            "discover",
            "--aggregation-type",
            "byproperty",
            "--data-state",
            "all",
            "--limit",
            "5",
        ],
    );

    assert_eq!(out.code, 0, "{out:?}");
    // Google leaves `rows` out when nothing matches.
    assert_eq!(out.json()["rows"], json!([]));
    assert_eq!(out.json()["responseAggregationType"], Value::Null);
}

#[test]
fn without_an_end_date_the_probe_finds_the_latest_day() {
    let server = Server::run();
    server.expect(expect_probe().respond_with(status_code(200).body(probe_answer("2026-11-05"))));
    server.expect(
        expect_post(
            QUERY_PATH,
            json!({"startDate": "2026-10-30", "endDate": "2026-11-05", "type": "web", "dataState": "final", "rowLimit": 1001, "startRow": 0}),
        )
        .respond_with(status_code(200).body(rows(&[]))),
    );

    let out = run_api(&server, &["query", "-s", SITE, "--days", "7"]);

    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.json()["request"]["endDate"], "2026-11-05");
}

#[test]
fn hourly_data_probes_with_all() {
    let server = Server::run();
    server.expect(
        expect_query_with("dataState", json!("all"))
            .respond_with(status_code(200).body(probe_answer("2026-11-07"))),
    );
    server.expect(
        expect_query_with("dataState", json!("hourly_all"))
            .respond_with(status_code(200).body(rows(&[]))),
    );

    let out = run_api(
        &server,
        &[
            "query",
            "-s",
            SITE,
            "-d",
            "hour",
            "--data-state",
            "hourly_all",
            "--days",
            "1",
        ],
    );

    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.json()["request"]["startDate"], "2026-11-07");
}

#[test]
fn a_probe_without_rows_warns_and_ends_three_days_ago() {
    let server = Server::run();
    server.expect(expect_probe().respond_with(status_code(200).body("{}")));
    server.expect(
        expect_query_with("startRow", json!(0)).respond_with(status_code(200).body(rows(&[]))),
    );

    let out = run_api(&server, &["query", "-s", SITE]);

    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.warnings(), ["no_recent_data"]);
    let end = time::OffsetDateTime::now_utc().date() - time::Duration::days(3);
    let end = format!(
        "{:04}-{:02}-{:02}",
        end.year(),
        u8::from(end.month()),
        end.day()
    );
    assert_eq!(out.json()["request"]["endDate"], end);
}

#[test]
fn a_start_before_the_16_months_warns() {
    let server = Server::run();
    server.expect(
        expect_query_with("startDate", json!("2020-01-01"))
            .respond_with(status_code(200).body(rows(&[]))),
    );

    let out = run_api(
        &server,
        &[
            "query",
            "-s",
            SITE,
            "--start-date",
            "2020-01-01",
            "--days",
            "3",
        ],
    );

    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.warnings(), ["before_retention"]);
    // Google keeps some days more than 16 months: the live API returned rows
    // 12 days before the cutoff. So the warning must not say none come back.
    let lines = out.stderr_lines();
    let message = lines[0]["warning"]["message"].as_str().unwrap();
    assert!(
        message.starts_with("Search Console keeps about 16 months of data")
            && message.ends_with("may be missing"),
        "{message}"
    );
}

#[test]
fn a_limit_asks_for_one_more_row_and_warns_when_more_match() {
    let server = Server::run();
    let three = [row(&["/a"], 3, 9), row(&["/b"], 2, 9), row(&["/c"], 1, 9)];
    server.expect(
        expect_post(
            QUERY_PATH,
            body(&json!({"dimensions": ["page"], "rowLimit": 3})),
        )
        .respond_with(status_code(200).body(rows(&three))),
    );

    let out = query(&server, &["-d", "page", "--limit", "2"]);

    assert_eq!(out.code, 0, "{out:?}");
    let json = out.json();
    assert_eq!(json["rowCount"], 2);
    assert_eq!(json["truncated"], true);
    assert_eq!(json["rows"][1]["page"], "/b");
    assert_eq!(out.warnings(), ["truncated"]);
}

#[test]
fn all_pages_through_every_row() {
    let server = Server::run();
    let page: Vec<Value> = (0..25_000)
        .map(|i| row(&[&format!("/p{i}")], 1, 2))
        .collect();
    server.expect(
        expect_post(
            QUERY_PATH,
            body(&json!({"dimensions": ["page"], "rowLimit": 25_000})),
        )
        .respond_with(status_code(200).body(rows(&page))),
    );
    server.expect(
        expect_post(
            QUERY_PATH,
            body(&json!({"dimensions": ["page"], "rowLimit": 25_000, "startRow": 25_000})),
        )
        .respond_with(status_code(200).body(rows(&[row(&["/last"], 1, 2)]))),
    );

    let out = query(&server, &["-d", "page", "--all", "--format", "jsonl"]);

    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stderr, "");
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert_eq!(lines.len(), 25_001);
    assert_eq!(
        lines[25_000],
        r#"{"page":"/last","clicks":1,"impressions":2,"ctr":0.5,"position":3.5}"#
    );
}

#[test]
fn jsonl_streams_rows_and_warns_on_truncation() {
    let server = Server::run();
    server.expect(
        expect_post(
            QUERY_PATH,
            body(&json!({"dimensions": ["query"], "rowLimit": 2})),
        )
        .respond_with(status_code(200).body(rows(&[row(&["a"], 2, 4), row(&["b"], 1, 4)]))),
    );

    let out = query(
        &server,
        &["-d", "query", "--limit", "1", "--format", "jsonl"],
    );

    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.json_lines(),
        [json!({"query": "a", "clicks": 2, "impressions": 4, "ctr": 0.5, "position": 3.5})]
    );
    assert_eq!(out.warnings(), ["truncated"]);
}

#[test]
fn csv_has_a_header_and_quotes_what_needs_it() {
    let server = Server::run();
    server.expect(
        expect_post(QUERY_PATH, body(&json!({"dimensions": ["query", "date"]}))).respond_with(
            status_code(200).body(rows(&[
                row(&["shoes, red", "2026-10-02"], 1, 4),
                row(&["say \"hi\"", "2026-10-03"], 0, 4),
            ])),
        ),
    );

    let out = query(&server, &["-d", "query,date", "--format", "csv"]);

    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stdout,
        "query,date,clicks,impressions,ctr,position\n\
         \"shoes, red\",2026-10-02,1,4,0.25,3.5\n\
         \"say \"\"hi\"\"\",2026-10-03,0,4,0.0,3.5\n"
    );
}

#[test]
fn sort_fetches_every_row_then_sorts_and_limits() {
    let server = Server::run();
    server.expect(
        expect_post(
            QUERY_PATH,
            body(&json!({"dimensions": ["page"], "rowLimit": 11})),
        )
        .respond_with(status_code(200).body(rows(&[
            row(&["/a"], 9, 10),
            row(&["/b"], 1, 500),
            row(&["/c"], 5, 90),
        ]))),
    );

    let out = query(
        &server,
        &[
            "-d",
            "page",
            "--sort",
            "impressions",
            "--limit",
            "2",
            "--max-rows",
            "10",
        ],
    );

    assert_eq!(out.code, 0, "{out:?}");
    let pages: Vec<Value> = out.json()["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["page"].clone())
        .collect();
    assert_eq!(pages, [json!("/b"), json!("/c")]);
    assert_eq!(out.warnings(), ["truncated"]);
}

#[test]
fn sort_caps_the_fetch_at_max_rows_and_warns() {
    let server = Server::run();
    server.expect(
        expect_post(
            QUERY_PATH,
            body(&json!({"dimensions": ["page"], "rowLimit": 3})),
        )
        .respond_with(status_code(200).body(rows(&[
            row(&["/a"], 9, 10),
            row(&["/b"], 1, 500),
            row(&["/c"], 1, 5),
        ]))),
    );

    let out = query(
        &server,
        &[
            "-d",
            "page",
            "--sort",
            "page",
            "--order",
            "asc",
            "--max-rows",
            "2",
            "--all",
            "--format",
            "csv",
        ],
    );

    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stdout,
        "page,clicks,impressions,ctr,position\n/a,9,10,0.9,3.5\n/b,1,500,0.002,3.5\n"
    );
    assert_eq!(out.warnings(), ["row_cap"]);
}

#[test]
fn an_unknown_sort_column_is_a_config_error_before_any_request() {
    let server = Server::run();
    let out = query(&server, &["-d", "page", "--sort", "query"]);
    assert_eq!(out.code, 2, "{out:?}");
    let error = out.error();
    assert_eq!(error["kind"], "config");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("page, clicks, impressions, ctr, position"),
        "{error}"
    );
}

#[test]
fn bad_flags_are_usage_errors() {
    let server = Server::run();
    for args in [
        vec!["--start-date", "2026-10-28", "--end-date", "2026-10-01"],
        vec![
            "--start-date",
            "2026-10-01",
            "--end-date",
            "2026-10-02",
            "--days",
            "2",
        ],
    ] {
        let mut all = vec!["query", "-s", SITE];
        all.extend(args);
        let out = run_api(&server, &all);
        assert_eq!(out.code, 2, "{out:?}");
        assert_eq!(out.error()["kind"], "config");
    }
    for args in [
        vec!["--start-date", "2026-13-01"],
        vec!["--filter", "page"],
        vec!["--days", "0"],
        vec!["--limit", "0"],
        vec!["--limit", "5", "--all"],
        vec!["--order", "asc"],
        vec!["-d", "nope"],
    ] {
        let mut all = vec!["query", "-s", SITE];
        all.extend(args.iter().copied());
        let out = run_api(&server, &all);
        assert_eq!(out.code, 2, "{args:?}: {out:?}");
    }
}

#[test]
fn a_start_after_the_latest_day_is_a_config_error() {
    let server = Server::run();
    server.expect(expect_probe().respond_with(status_code(200).body(probe_answer("2026-11-05"))));
    let out = run_api(
        &server,
        &["query", "-s", SITE, "--start-date", "2026-11-06"],
    );
    assert_eq!(out.code, 2, "{out:?}");
    assert!(
        out.error()["message"]
            .as_str()
            .unwrap()
            .contains("latest day")
    );
}

#[test]
fn dry_run_shows_the_probe_and_the_first_page_without_credentials() {
    let out = common::run(
        &[
            "query",
            "-s",
            "https://Example.com",
            "--days",
            "3",
            "--all",
            "--dry-run",
            "--format",
            "jsonl",
        ],
        &[],
    );
    assert_eq!(out.code, 0, "{out:?}");
    let requests = out.json_lines();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0]["url"],
        "https://searchconsole.googleapis.com/webmasters/v3/sites/https%3A%2F%2Fexample.com%2F/searchAnalytics/query"
    );
    assert_eq!(requests[0]["body"]["dimensions"], json!(["date"]));
    assert_eq!(
        requests[1]["body"]["startDate"],
        "<latest day with data - 2 days>"
    );
    assert_eq!(requests[1]["body"]["rowLimit"], 25_000);
}
