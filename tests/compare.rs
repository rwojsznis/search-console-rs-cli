//! `compare`: the four requests, the join, the totals, the sort and the
//! formats (decision 11).

mod common;

use common::{QUERY_PATH, SITE, expect_post, expect_probe, probe_answer, row, rows, run_api};
use httptest::Server;
use httptest::responders::status_code;
use serde_json::{Value, json};

fn totals(start: &str, end: &str) -> Value {
    json!({"startDate": start, "endDate": end, "type": "web", "dataState": "final"})
}

fn grouped(start: &str, end: &str) -> Value {
    json!({
        "startDate": start, "endDate": end, "dimensions": ["page"], "type": "web",
        "dataState": "final", "rowLimit": 25_000, "startRow": 0,
    })
}

fn total_row(clicks: u64, impressions: u64) -> String {
    json!({"rows": [{"clicks": clicks, "impressions": impressions, "ctr": 0.1, "position": 4.0}]})
        .to_string()
}

/// The four requests of a grouped comparison of October with September.
fn expect_four(server: &Server, now: &[Value], before: &[Value]) {
    for (body, answer) in [
        (totals("2026-10-01", "2026-10-28"), total_row(100, 1000)),
        (totals("2026-09-03", "2026-09-30"), total_row(80, 900)),
        (grouped("2026-10-01", "2026-10-28"), rows(now)),
        (grouped("2026-09-03", "2026-09-30"), rows(before)),
    ] {
        server.expect(expect_post(QUERY_PATH, body).respond_with(status_code(200).body(answer)));
    }
}

const ARGS: [&str; 8] = [
    "compare",
    "-s",
    SITE,
    "-d",
    "page",
    "--start-date",
    "2026-10-01",
    "--end-date",
];

fn compare(server: &Server, extra: &[&str]) -> common::Run {
    let mut args = ARGS.to_vec();
    args.push("2026-10-28");
    args.extend(extra);
    run_api(server, &args)
}

#[test]
fn rows_are_joined_on_their_keys_with_the_totals_in_the_envelope() {
    let server = Server::run();
    expect_four(
        &server,
        &[row(&["/a"], 10, 100), row(&["/new"], 4, 10)],
        &[row(&["/a"], 4, 50), row(&["/gone"], 7, 70)],
    );

    let out = compare(&server, &["--sort", "clicksChange", "--order", "asc"]);

    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stderr, "");
    let json = out.json();
    assert_eq!(json["siteUrl"], SITE);
    assert_eq!(
        json["current"],
        json!({"startDate": "2026-10-01", "endDate": "2026-10-28"})
    );
    assert_eq!(
        json["previous"],
        json!({"startDate": "2026-09-03", "endDate": "2026-09-30"})
    );
    assert_eq!(
        json["request"],
        json!({"dimensions": ["page"], "type": "web", "dataState": "final"})
    );
    assert_eq!(json["totals"]["clicksChange"], 20);
    assert_eq!(json["totals"]["impressionsPrevious"], 900);
    let pages: Vec<Value> = json["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["page"].clone())
        .collect();
    assert_eq!(pages, [json!("/gone"), json!("/new"), json!("/a")]);
    assert_eq!(json["rows"][0]["clicks"], 0);
    assert_eq!(json["rows"][0]["clicksChange"], -7);
    assert_eq!(json["rows"][0]["position"], Value::Null);
    assert_eq!(json["rows"][2]["clicksChange"], 6);
    assert_eq!(json["rowCount"], 3);
    assert_eq!(json["truncated"], false);
}

#[test]
fn csv_has_every_column_and_a_limit_warns() {
    let server = Server::run();
    expect_four(
        &server,
        &[row(&["/a"], 10, 100), row(&["/b"], 1, 10)],
        &[row(&["/a"], 5, 100)],
    );

    let out = compare(&server, &["--format", "csv", "--limit", "1"]);

    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stdout,
        "page,clicks,clicksPrevious,clicksChange,impressions,impressionsPrevious,impressionsChange,\
         ctr,ctrPrevious,ctrChange,position,positionPrevious,positionChange\n\
         /a,10,5,5,100,100,0,0.1,0.05,0.05,3.5,3.5,0\n"
    );
    assert_eq!(out.warnings(), ["truncated"]);
}

#[test]
fn jsonl_prints_one_row_per_line() {
    let server = Server::run();
    expect_four(&server, &[row(&["/a"], 1, 2)], &[]);
    let out = compare(&server, &["--format", "jsonl", "--all"]);
    assert_eq!(out.code, 0, "{out:?}");
    let lines = out.json_lines();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["clicksPrevious"], 0);
}

#[test]
fn without_dimensions_the_totals_are_the_one_row() {
    let server = Server::run();
    server.expect(
        expect_post(QUERY_PATH, totals("2026-10-01", "2026-10-07"))
            .respond_with(status_code(200).body(total_row(10, 100))),
    );
    server.expect(
        expect_post(QUERY_PATH, totals("2025-10-02", "2025-10-08"))
            .respond_with(status_code(200).body("{}")),
    );

    let out = run_api(
        &server,
        &[
            "compare",
            "-s",
            SITE,
            "--end-date",
            "2026-10-07",
            "--days",
            "7",
            "--yoy",
            "--format",
            "jsonl",
        ],
    );

    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.json_lines(),
        [json!({
            "clicks": 10, "clicksPrevious": 0, "clicksChange": 10,
            "impressions": 100, "impressionsPrevious": 0, "impressionsChange": 100,
            "ctr": 0.1, "ctrPrevious": null, "ctrChange": null,
            "position": 4.0, "positionPrevious": null, "positionChange": null,
        })]
    );
}

#[test]
fn the_previous_period_can_start_anywhere_and_old_ones_warn() {
    let server = Server::run();
    server.expect(expect_probe().respond_with(status_code(200).body(probe_answer("2026-11-05"))));
    server.expect(
        expect_post(QUERY_PATH, totals("2026-11-04", "2026-11-05"))
            .respond_with(status_code(200).body(total_row(1, 1))),
    );
    server.expect(
        expect_post(QUERY_PATH, totals("2020-01-01", "2020-01-02"))
            .respond_with(status_code(200).body(total_row(1, 1))),
    );

    let out = run_api(
        &server,
        &[
            "compare",
            "-s",
            SITE,
            "--days",
            "2",
            "--previous-start-date",
            "2020-01-01",
        ],
    );

    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.warnings(), ["before_retention"]);
    assert_eq!(out.json()["previous"]["endDate"], "2020-01-02");
}

#[test]
fn a_capped_period_warns() {
    let server = Server::run();
    for (body, answer) in [
        (totals("2026-10-01", "2026-10-28"), total_row(1, 1)),
        (totals("2026-09-03", "2026-09-30"), total_row(1, 1)),
    ] {
        server.expect(expect_post(QUERY_PATH, body).respond_with(status_code(200).body(answer)));
    }
    let mut capped = grouped("2026-10-01", "2026-10-28");
    capped["rowLimit"] = json!(2);
    server.expect(
        expect_post(QUERY_PATH, capped)
            .respond_with(status_code(200).body(rows(&[row(&["/a"], 2, 2), row(&["/b"], 1, 2)]))),
    );
    let mut before = grouped("2026-09-03", "2026-09-30");
    before["rowLimit"] = json!(2);
    server.expect(expect_post(QUERY_PATH, before).respond_with(status_code(200).body("{}")));

    let out = compare(&server, &["--max-rows", "1"]);

    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.warnings(), ["row_cap"]);
    assert_eq!(out.json()["rowCount"], 1);
}

#[test]
fn an_unknown_sort_column_lists_the_columns() {
    let server = Server::run();
    let out = compare(&server, &["--sort", "clicksDelta"]);
    assert_eq!(out.code, 2, "{out:?}");
    assert!(
        out.error()["message"]
            .as_str()
            .unwrap()
            .contains("clicksChange"),
        "{out:?}"
    );
}

#[test]
fn yoy_and_previous_start_date_conflict() {
    let server = Server::run();
    let out = compare(&server, &["--yoy", "--previous-start-date", "2025-01-01"]);
    assert_eq!(out.code, 2, "{out:?}");
}

#[test]
fn dry_run_lists_the_five_requests() {
    let out = common::run(&["compare", "-s", SITE, "-d", "query", "--dry-run"], &[]);
    assert_eq!(out.code, 0, "{out:?}");
    let requests = out.json()["requests"].as_array().unwrap().clone();
    let bodies: Vec<(Value, Value)> = requests
        .iter()
        .map(|r| (r["body"]["startDate"].clone(), r["body"]["endDate"].clone()))
        .collect();
    assert_eq!(bodies.len(), 5);
    assert_eq!(
        bodies[1..],
        [
            (
                json!("<latest day with data - 27 days>"),
                json!("<latest day with data>")
            ),
            (
                json!("<latest day with data - 55 days>"),
                json!("<latest day with data - 28 days>")
            ),
            (
                json!("<latest day with data - 27 days>"),
                json!("<latest day with data>")
            ),
            (
                json!("<latest day with data - 55 days>"),
                json!("<latest day with data - 28 days>")
            ),
        ]
    );
    let open = common::run(
        &[
            "compare",
            "-s",
            SITE,
            "--start-date",
            "2026-01-01",
            "--dry-run",
            "--format",
            "csv",
        ],
        &[],
    );
    assert_eq!(open.code, 0, "{open:?}");
    let lines = open.json_lines();
    assert_eq!(
        lines[2]["body"]["startDate"],
        "<known after the latest day with data>"
    );
}
