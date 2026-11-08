//! Smoke tests against the real Search Console API. Opt-in: every test is
//! `#[ignore]`, CI never runs them, and each one skips when `GSC_LIVE_SITE`
//! is not set.
//!
//! They use your own credentials, found the usual way (decision 6). Set
//! them, `GSC_LIVE_SITE` and, for the URL Inspection test, `GSC_LIVE_URL` (a
//! page of that property) in `mise.local.toml` (from
//! `mise.local.toml.example`), then:
//!
//! ```bash
//! mise exec -- cargo test --test live -- --ignored
//! ```
//! They are safe on a production property: every command reads (decision 1),
//! and the queries ask for a few rows.

use serde_json::Value;
use std::process::Command;

/// Runs the command with the developer's own environment, and returns the
/// parsed stdout. `None` when `GSC_LIVE_SITE` is not set.
fn live(args: &[&str]) -> Option<Value> {
    let Ok(site) = std::env::var("GSC_LIVE_SITE") else {
        eprintln!("skipped: set GSC_LIVE_SITE to run this test");
        return None;
    };
    let out = Command::new(env!("CARGO_BIN_EXE_gsc-cli"))
        .args(args)
        .env("GSC_SITE", site)
        .output()
        .unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(out.status.code(), Some(0), "{args:?}: {stderr}");
    Some(serde_json::from_slice(&out.stdout).unwrap())
}

#[test]
#[ignore = "live: needs GSC_LIVE_SITE and credentials"]
fn auth_status() {
    if let Some(status) = live(&["auth", "status"]) {
        assert!(status["sites"].as_u64().unwrap() > 0, "{status}");
    }
}

#[test]
#[ignore = "live: needs GSC_LIVE_SITE and credentials"]
fn sites_list_holds_the_live_site() {
    if let Some(sites) = live(&["sites", "list"]) {
        let site = std::env::var("GSC_LIVE_SITE").unwrap();
        let names: Vec<&str> = sites["siteEntry"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|s| s["siteUrl"].as_str())
            .collect();
        assert!(names.contains(&site.as_str()), "{names:?}");
    }
}

#[test]
#[ignore = "live: needs GSC_LIVE_SITE and credentials"]
fn query_totals_and_pages() {
    if let Some(totals) = live(&["query", "--days", "7"]) {
        assert_eq!(totals["rowCount"], 1, "{totals}");
    }
    if let Some(pages) = live(&["query", "-d", "page,query", "--limit", "3"]) {
        assert!(pages["rows"].as_array().unwrap().len() <= 3, "{pages}");
    }
}

#[test]
#[ignore = "live: needs GSC_LIVE_SITE and credentials"]
fn compare_weeks() {
    if let Some(compare) = live(&["compare", "-d", "page", "--days", "7", "--limit", "3"]) {
        assert!(compare["totals"]["clicks"].is_number(), "{compare}");
    }
}

#[test]
#[ignore = "live: needs GSC_LIVE_SITE and credentials"]
fn sitemaps_list() {
    if let Some(sitemaps) = live(&["sitemaps", "list"]) {
        assert!(sitemaps.is_object(), "{sitemaps}");
    }
}

#[test]
#[ignore = "live: needs GSC_LIVE_SITE, GSC_LIVE_URL and credentials"]
fn inspect_a_page() {
    let Ok(url) = std::env::var("GSC_LIVE_URL") else {
        eprintln!("skipped: set GSC_LIVE_URL to run this test");
        return;
    };
    if let Some(result) = live(&["inspect", &url]) {
        assert!(result["inspectionResult"].is_object(), "{result}");
    }
}
