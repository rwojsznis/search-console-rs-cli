//! The registry, the clap tree, the read-only endpoint list, the help
//! footers and `docs/commands.md` must agree (decisions 1 and 13).

mod common;

use common::run;
use gsc_cli::api::ENDPOINTS;
use gsc_cli::cli::catalog::registry;
use gsc_cli::cli::command;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt::Write;

/// Every leaf of the clap tree, as its command path.
fn leaves(cmd: &clap::Command, prefix: &[String], out: &mut Vec<Vec<String>>) {
    for sub in cmd.get_subcommands() {
        let mut path = prefix.to_vec();
        path.push(sub.get_name().to_owned());
        if sub.has_subcommands() {
            leaves(sub, &path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn every_leaf_is_in_the_registry_and_every_entry_is_a_leaf() {
    let mut tree = Vec::new();
    leaves(&command(), &[], &mut tree);
    let tree: BTreeSet<Vec<String>> = tree.into_iter().filter(|p| p != &["commands"]).collect();
    let registered: BTreeSet<Vec<String>> = registry()
        .iter()
        .map(|(path, _)| path.iter().map(|s| (*s).to_owned()).collect())
        .collect();
    assert_eq!(tree, registered);
}

#[test]
fn every_command_calls_only_the_read_only_endpoints_and_each_has_a_command() {
    let mut used = BTreeSet::new();
    for (path, spec) in registry() {
        for endpoint in spec.endpoints {
            assert!(ENDPOINTS.contains(endpoint), "{path:?}: {endpoint:?}");
            used.insert(endpoint.path);
        }
    }
    let all: BTreeSet<&str> = ENDPOINTS.iter().map(|e| e.path).collect();
    assert_eq!(used, all);
}

/// Whether `path` fills `template`: each `{...}` is one non-empty segment.
fn fills(template: &str, path: &str) -> bool {
    let (template, path): (Vec<&str>, Vec<&str>) =
        (template.split('/').collect(), path.split('/').collect());
    template.len() == path.len()
        && template.iter().zip(&path).all(|(want, got)| {
            if want.starts_with('{') {
                !got.is_empty()
            } else {
                want == got
            }
        })
}

#[test]
fn every_example_dry_runs_to_its_own_endpoints() {
    for (path, spec) in registry().iter().filter(|(_, spec)| spec.dry_run) {
        let mut args: Vec<&str> = spec.example.split_whitespace().skip(1).collect();
        args.push("--dry-run");
        let out = run(&args, &[]);
        assert_eq!(out.code, 0, "{path:?}: {out:?}");
        for request in out.json()["requests"].as_array().unwrap() {
            let url = url::Url::parse(request["url"].as_str().unwrap()).unwrap();
            let method = request["method"].as_str().unwrap();
            assert!(
                spec.endpoints
                    .iter()
                    .any(|e| e.method.as_str() == method && fills(e.path, url.path())),
                "{path:?}: {method} {url}"
            );
        }
    }
}

#[test]
fn csv_is_refused_where_the_output_is_not_rows() {
    for (path, spec) in registry() {
        let mut args: Vec<&str> = spec.example.split_whitespace().skip(1).collect();
        args.extend(["--dry-run", "--format", "csv"]);
        let out = run(&args, &[]);
        if spec.tabular {
            assert_eq!(out.code, 0, "{path:?}: {out:?}");
        } else {
            assert_eq!(out.code, 2, "{path:?}: {out:?}");
            assert!(
                out.error()["message"].as_str().unwrap().contains("csv"),
                "{path:?}"
            );
        }
    }
}

#[test]
fn every_leaf_help_ends_with_its_requests_and_example() {
    for (path, spec) in registry() {
        let mut args = path.to_vec();
        args.push("--help");
        let out = run(&args, &[]);
        assert_eq!(out.code, 0, "{path:?}");
        assert!(
            out.stdout
                .contains(&format!("Example:  {}\n", spec.example)),
            "{path:?}: {}",
            out.stdout
        );
        for endpoint in spec.endpoints {
            assert!(out.stdout.contains(endpoint.path), "{path:?}");
        }
    }
}

#[test]
fn the_catalog_describes_every_command_from_the_parser() {
    let out = run(&["commands"], &[]);
    assert_eq!(out.code, 0, "{out:?}");
    let catalog = out.json();
    assert_eq!(catalog["read_only"], true);
    assert_eq!(catalog["version"], "0.0.0");
    let commands = catalog["commands"].as_array().unwrap();
    assert_eq!(commands.len(), registry().len());
    let query = &commands[0];
    assert_eq!(query["command"], "query");
    assert_eq!(
        query["formats"],
        serde_json::json!(["json", "jsonl", "csv"])
    );
    let dimensions = query["args"]
        .as_array()
        .unwrap()
        .iter()
        .find(|arg| arg["long"] == "--dimensions")
        .unwrap();
    assert_eq!(dimensions["multiple"], true);
    assert!(
        dimensions["possible_values"]
            .as_array()
            .unwrap()
            .contains(&Value::from("searchAppearance"))
    );
    let site = query["args"]
        .as_array()
        .unwrap()
        .iter()
        .find(|arg| arg["long"] == "--site")
        .unwrap();
    assert_eq!(site["env"], "GSC_SITE");
    let token = catalog["global_args"]
        .as_array()
        .unwrap()
        .iter()
        .find(|arg| arg["long"] == "--access-token")
        .unwrap();
    assert_eq!(token["env"], "GSC_ACCESS_TOKEN");

    let lines = run(&["commands", "--format", "jsonl"], &[]);
    assert_eq!(lines.json_lines().len(), registry().len());
}

/// `docs/commands.md`, from the catalog.
fn commands_md() -> String {
    let catalog = run(&["commands"], &[]).json();
    let mut md = String::from(
        "# Commands\n\n\
         Generated by `tests/catalog.rs` from the command registry. After changing a\n\
         command, run `UPDATE_DOCS=1 cargo test --test catalog docs_commands` and\n\
         review the diff. `gsc-cli commands` prints the same as JSON, with every\n\
         flag.\n\n\
         Every command is read-only. Each one also calls the OAuth token endpoint\n\
         first, unless it is given an access token.\n\n\
         | Command | Requests | Formats | Summary |\n\
         |---|---|---|---|\n",
    );
    for command in catalog["commands"].as_array().unwrap() {
        let requests: Vec<String> = command["requests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| format!("`{}`", r.as_str().unwrap()))
            .collect();
        let formats: Vec<&str> = command["formats"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f.as_str().unwrap())
            .collect();
        writeln!(
            md,
            "| `{}` | {} | {} | {} |",
            command["command"].as_str().unwrap(),
            requests.join("<br>"),
            formats.join(", "),
            command["summary"].as_str().unwrap()
        )
        .unwrap();
    }
    md.push_str("\n## Examples\n\n```bash\n");
    for command in catalog["commands"].as_array().unwrap() {
        writeln!(md, "{}", command["example"].as_str().unwrap()).unwrap();
    }
    md.push_str("```\n");
    md
}

#[test]
fn docs_commands_md_is_generated_from_the_catalog() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/commands.md");
    let want = commands_md();
    if std::env::var_os("UPDATE_DOCS").is_some() {
        std::fs::write(path, &want).unwrap();
    }
    let have = std::fs::read_to_string(path).unwrap_or_default();
    assert!(
        have == want,
        "docs/commands.md is out of date: run `UPDATE_DOCS=1 cargo test --test catalog \
         docs_commands` and review the diff"
    );
}
