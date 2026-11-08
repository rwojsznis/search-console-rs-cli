//! `sites`, `sitemaps` and `inspect`: one request each, and Google's JSON
//! unchanged on stdout (decision 3).

mod common;

use common::{SITE, SITE_PATH, expect_get, expect_post, run_api};
use httptest::Server;
use httptest::responders::status_code;
use serde_json::json;

#[test]
fn sites_list_prints_googles_body_unchanged() {
    let server = Server::run();
    let body = r#"{"siteEntry":[{"siteUrl":"sc-domain:example.com","permissionLevel":"siteOwner"},{"siteUrl":"https://example.com/","permissionLevel":"siteRestrictedUser"}]}"#;
    server.expect(
        expect_get("/webmasters/v3/sites", "")
            .times(2)
            .respond_with(status_code(200).body(body)),
    );

    let out = run_api(&server, &["sites", "list"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(
        out.stdout,
        format!(
            "{:#}\n",
            serde_json::from_str::<serde_json::Value>(body).unwrap()
        )
    );

    let lines = run_api(&server, &["sites", "list", "--format", "jsonl"]);
    assert_eq!(
        lines.stdout,
        "{\"siteUrl\":\"sc-domain:example.com\",\"permissionLevel\":\"siteOwner\"}\n\
         {\"siteUrl\":\"https://example.com/\",\"permissionLevel\":\"siteRestrictedUser\"}\n"
    );
}

#[test]
fn an_empty_list_prints_nothing_in_jsonl() {
    let server = Server::run();
    server.expect(expect_get("/webmasters/v3/sites", "").respond_with(status_code(200).body("{}")));
    let out = run_api(&server, &["sites", "list", "--format", "jsonl"]);
    assert_eq!((out.code, out.stdout.as_str()), (0, ""), "{out:?}");
}

#[test]
fn sites_get_encodes_the_site_as_one_segment() {
    let server = Server::run();
    server.expect(
        expect_get("/webmasters/v3/sites/https%3A%2F%2Fexample.com%2F", "")
            .respond_with(status_code(200).body(r#"{"siteUrl":"https://example.com/"}"#)),
    );
    let out = run_api(&server, &["sites", "get", "--site", "https://EXAMPLE.com"]);
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.json(), json!({"siteUrl": "https://example.com/"}));
}

#[test]
fn the_site_can_come_from_gsc_site() {
    let server = Server::run();
    server.expect(expect_get(SITE_PATH, "").respond_with(status_code(200).body("{}")));
    let out = common::run(
        &["sites", "get", "--base-url", &common::base_url(&server)],
        &[
            ("GSC_ACCESS_TOKEN", common::SENTINEL_TOKEN),
            ("GSC_SITE", SITE),
        ],
    );
    assert_eq!(out.code, 0, "{out:?}");
}

#[test]
fn sitemaps_list_and_get() {
    let server = Server::run();
    let list = r#"{"sitemap":[{"path":"https://example.com/sitemap.xml","errors":"0","warnings":"1","contents":[{"type":"web","submitted":"120","indexed":"0"}]}]}"#;
    server.expect(
        expect_get(
            &format!("{SITE_PATH}/sitemaps"),
            "sitemapIndex=https%3A%2F%2Fexample.com%2Findex.xml",
        )
        .respond_with(status_code(200).body(list)),
    );
    server.expect(
        expect_get(
            &format!("{SITE_PATH}/sitemaps/https%3A%2F%2Fexample.com%2Fsitemap.xml"),
            "",
        )
        .respond_with(status_code(200).body(r#"{"path":"https://example.com/sitemap.xml"}"#)),
    );

    let out = run_api(
        &server,
        &[
            "sitemaps",
            "list",
            "-s",
            SITE,
            "--sitemap-index",
            "https://example.com/index.xml",
            "--format",
            "jsonl",
        ],
    );
    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.json_lines()[0]["warnings"], "1");

    let one = run_api(
        &server,
        &[
            "sitemaps",
            "get",
            "-s",
            SITE,
            "https://example.com/sitemap.xml",
        ],
    );
    assert_eq!(one.code, 0, "{one:?}");
    assert_eq!(one.json()["path"], "https://example.com/sitemap.xml");
}

#[test]
fn inspect_posts_the_url_and_the_site() {
    let server = Server::run();
    let result = json!({"inspectionResult": {"indexStatusResult": {"verdict": "PASS", "coverageState": "Submitted and indexed"}}});
    server.expect(
        expect_post(
            "/v1/urlInspection/index:inspect",
            json!({"inspectionUrl": "https://example.com/pricing", "siteUrl": SITE, "languageCode": "en-US"}),
        )
        .respond_with(status_code(200).body(result.to_string())),
    );

    let out = run_api(
        &server,
        &[
            "inspect",
            "-s",
            SITE,
            "https://example.com/pricing",
            "--language-code",
            "en-US",
            "--format",
            "jsonl",
        ],
    );

    assert_eq!(out.code, 0, "{out:?}");
    assert_eq!(out.stdout, format!("{result}\n"));
}

#[test]
fn urls_must_be_http_or_https() {
    let server = Server::run();
    for args in [
        vec!["inspect", "-s", SITE, "example.com/a"],
        vec!["sitemaps", "get", "-s", SITE, "sitemap.xml"],
        vec!["sitemaps", "list", "-s", SITE, "--sitemap-index", "x"],
    ] {
        let out = run_api(&server, &args);
        assert_eq!(out.code, 2, "{args:?}: {out:?}");
    }
}
