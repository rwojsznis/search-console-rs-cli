//! The one list of leaf commands, and everything generated from it: each
//! leaf's `--help` footer and the `commands` catalog (decision 13). `args` in
//! the catalog come from clap's own `Command`, so they cannot drift from the
//! parser.

use crate::cli::commands::{auth, compare, inspect, query, sitemaps, sites};
use crate::cli::spec::CommandSpec;
use clap::Arg;
use serde_json::{Value, json};

type Entry = (&'static [&'static str], &'static CommandSpec);

static REGISTRY: &[Entry] = &[
    (&["query"], &query::SPEC),
    (&["compare"], &compare::SPEC),
    (&["sites", "list"], &sites::LIST),
    (&["sites", "get"], &sites::GET),
    (&["sitemaps", "list"], &sitemaps::LIST),
    (&["sitemaps", "get"], &sitemaps::GET),
    (&["inspect"], &inspect::SPEC),
    (&["auth", "login"], &auth::LOGIN),
    (&["auth", "status"], &auth::STATUS),
];

/// Every leaf command: its path in the command tree, and its spec.
#[must_use]
pub fn registry() -> &'static [Entry] {
    REGISTRY
}

/// The requests a command sends, as `METHOD /path` lines.
fn requests(spec: &CommandSpec) -> Vec<String> {
    let mut lines: Vec<String> = spec
        .endpoints
        .iter()
        .map(|e| format!("{} {}", e.method.as_str(), e.path))
        .collect();
    if spec.token_endpoint {
        lines.insert(0, "POST <token_uri> (the OAuth token endpoint)".to_owned());
    }
    lines
}

fn formats(spec: &CommandSpec) -> &'static [&'static str] {
    if spec.tabular {
        &["json", "jsonl", "csv"]
    } else {
        &["json", "jsonl"]
    }
}

/// The footer of a leaf's `--help`, from its spec.
pub(crate) fn leaf_help(spec: &CommandSpec) -> String {
    let mut lines: Vec<String> = requests(spec)
        .into_iter()
        .enumerate()
        .map(|(i, request)| {
            let label = if i == 0 { "Requests:" } else { "" };
            format!("{label:<10}{request}")
        })
        .collect();
    lines.push(format!("{:<10}{}", "Formats:", formats(spec).join(", ")));
    lines.push(format!("{:<10}{}", "Example:", spec.example));
    lines.join("\n")
}

/// The `commands` output: `{"version", "read_only", "global_args",
/// "commands"}`.
pub(crate) fn catalog(version: &str) -> Value {
    let mut cmd = crate::cli::command();
    // Building propagates the global flags and settles each arg's value
    // parser, which `possible_values` reads.
    cmd.build();
    let global: Vec<Value> = cmd
        .get_arguments()
        .filter(|arg| arg.is_global_set())
        .map(arg_json)
        .collect();
    let commands: Vec<Value> = REGISTRY
        .iter()
        .map(|(path, spec)| {
            let leaf = path
                .iter()
                .try_fold(&cmd, |cmd, name| cmd.find_subcommand(name));
            let args: Vec<Value> = leaf
                .into_iter()
                .flat_map(clap::Command::get_arguments)
                .filter(|arg| !arg.is_global_set() && !is_builtin(arg))
                .map(arg_json)
                .collect();
            json!({
                "command": path.join(" "),
                "summary": spec.summary,
                "requests": requests(spec),
                "formats": formats(spec),
                "list_key": spec.list_key,
                "example": spec.example,
                "args": args,
            })
        })
        .collect();
    json!({
        "version": version,
        "read_only": true,
        "global_args": global,
        "commands": commands,
    })
}

/// `--help` and `--version`: clap adds them to every command.
fn is_builtin(arg: &Arg) -> bool {
    matches!(arg.get_id().as_str(), "help" | "version")
}

fn arg_json(arg: &Arg) -> Value {
    let help = arg.get_long_help().or_else(|| arg.get_help());
    let possible: Vec<String> = arg
        .get_possible_values()
        .iter()
        .filter(|value| !value.is_hide_set())
        .map(|value| value.get_name().to_owned())
        .collect();
    json!({
        "id": arg.get_id().as_str(),
        "long": arg.get_long().map(|long| format!("--{long}")),
        "positional": arg.is_positional(),
        "required": arg.is_required_set(),
        "takes_value": arg.get_action().takes_values(),
        "multiple": matches!(arg.get_action(), clap::ArgAction::Append),
        "help": help.map(ToString::to_string),
        "possible_values": possible,
        "default": arg.get_default_values().first().map(|v| v.to_string_lossy()),
        "env": arg.get_env().map(|env| env.to_string_lossy()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_help_footer_names_the_requests_formats_and_example() {
        assert_eq!(
            leaf_help(&query::SPEC),
            format!(
                "Requests: POST /webmasters/v3/sites/{{siteUrl}}/searchAnalytics/query\n\
                 Formats:  json, jsonl, csv\n\
                 Example:  {}",
                query::SPEC.example
            )
        );
        assert_eq!(
            leaf_help(&auth::STATUS),
            "Requests: POST <token_uri> (the OAuth token endpoint)\n          \
             GET /webmasters/v3/sites\n\
             Formats:  json, jsonl\n\
             Example:  gsc-cli auth status"
        );
    }
}
