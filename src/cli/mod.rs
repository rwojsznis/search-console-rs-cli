//! The command line: the global flags, the command tree, and the one
//! dispatcher every leaf goes through (decision 13).

pub mod catalog;
pub(crate) mod commands;
pub mod exit;
pub mod out;
pub mod spec;

use crate::api::{Api, DEFAULT_BASE_URL, parse_base_url};
use crate::auth::{AuthConfig, Env};
use crate::cli::commands::auth::AuthCommand;
use crate::cli::commands::compare::Compare;
use crate::cli::commands::inspect::Inspect;
use crate::cli::commands::query::Query;
use crate::cli::commands::sitemaps::SitemapsCommand;
use crate::cli::commands::sites::SitesCommand;
use crate::cli::commands::{Context, Leaf};
use crate::cli::exit::ExitStatus;
use crate::client::Client;
use crate::error::Error;
use crate::http::Http;
use crate::output::{self, Format};
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand};
use std::path::PathBuf;
use std::time::Duration;

const VERSION: &str = match option_env!("GSC_CLI_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

const AFTER_HELP: &str = "\
Read-only: gsc-cli asks Google for the webmasters.readonly scope only, and
calls only endpoints that read. No command changes anything in Search
Console, so an agent can run any of them without asking first. (`auth login`
writes one local file: the credentials.)

Credentials, the first one set wins (`gsc-cli auth status` shows which):
  GSC_ACCESS_TOKEN                an access token (--access-token)
  GSC_CREDENTIALS                 a service account key or authorized_user
                                  file (--credentials)
  ~/.config/gsc-cli/credentials.json  written by `gsc-cli auth login`
  GOOGLE_APPLICATION_CREDENTIALS
  ~/.config/gcloud/application_default_credentials.json

Environment:
  GSC_SITE        the property for --site
  GSC_BASE_URL    the API root (--base-url)
  GSC_CONFIG_DIR  where `auth login` writes (default ~/.config/gsc-cli)

Exit codes:
  0  success, also when stdout closes early
  1  Search Console answered with an error status (error kind \"api\")
  2  usage or configuration error; nothing was sent (\"config\")
  3  no usable answer: connection, TLS, timeout, or a body that is not JSON
     (\"transport\", \"timeout\", \"decode\")
  4  no credentials, or Google refused them (\"auth\"): a person must act

Errors and warnings are one JSON line each on stderr. Dates are Pacific Time
days, as in Search Console.

Run `gsc-cli commands` for a JSON catalog of every command.";

// No `Debug`: `Global` holds the access token, and a derived `Debug` would
// print it (decision 6).
#[derive(Parser)]
#[command(
    name = "gsc-cli",
    about = "Unofficial, read-only Google Search Console client for LLM agents.",
    after_help = AFTER_HELP,
    version = VERSION
)]
struct Cli {
    #[command(flatten)]
    global: Global,
    #[command(subcommand)]
    command: Command,
}

/// The flags of decision 7. `global = true`, so they work before or after
/// the subcommand.
#[derive(Args)]
#[command(next_help_heading = "Global options")]
struct Global {
    /// An OAuth access token with the webmasters.readonly scope, used as
    /// given: `gcloud auth print-access-token`, for example.
    #[arg(
        long,
        env = "GSC_ACCESS_TOKEN",
        hide_env_values = true,
        global = true,
        value_name = "TOKEN"
    )]
    access_token: Option<String>,

    /// A credentials file: a service account key, or an `authorized_user`
    /// file (what `auth login` and gcloud write).
    #[arg(
        long,
        env = "GSC_CREDENTIALS",
        hide_env_values = true,
        global = true,
        value_name = "FILE"
    )]
    credentials: Option<PathBuf>,

    /// The API root. Must be http or https.
    // `hide_env_values` only for the help text: without it, clap prints
    // `[env: GSC_BASE_URL=]` when the variable is unset.
    #[arg(
        long,
        env = "GSC_BASE_URL",
        hide_env_values = true,
        default_value = DEFAULT_BASE_URL,
        global = true,
        value_name = "URL"
    )]
    base_url: String,

    /// Seconds to wait for each response.
    #[arg(
        long,
        default_value_t = 60,
        value_parser = clap::value_parser!(u64).range(1..=3600),
        global = true,
        value_name = "SECONDS"
    )]
    timeout: u64,

    /// Retries of a request that got 429 or 5xx, with backoff.
    #[arg(
        long,
        default_value_t = 2,
        value_parser = clap::value_parser!(u32).range(0..=10),
        global = true,
        value_name = "N"
    )]
    retries: u32,

    /// The stdout layout. csv works with query and compare.
    #[arg(long, value_enum, default_value_t = Format::Json, global = true)]
    format: Format,

    /// Print the requests instead of sending them. Needs no credentials.
    #[arg(long, global = true)]
    dry_run: bool,

    /// Log each request and its outcome on stderr, as JSON lines.
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Clicks, impressions, CTR and position, grouped by page, query,
    /// date, country, device or search appearance.
    Query(Box<Query>),
    /// Two periods side by side: what rose, what fell.
    Compare(Box<Compare>),
    /// The properties the credentials can read.
    #[command(subcommand)]
    Sites(SitesCommand),
    /// Submitted sitemaps: errors, warnings, counts.
    #[command(subcommand)]
    Sitemaps(SitemapsCommand),
    /// URL Inspection: Google's index status of one URL.
    Inspect(Inspect),
    /// Log in, and check the credentials.
    #[command(subcommand)]
    Auth(AuthCommand),
    /// Describe every command as JSON: requests, formats and flags. Needs no
    /// network and no credentials.
    Commands,
}

impl Command {
    /// The leaf, or `None` for `commands`.
    fn leaf(&self) -> Option<&dyn Leaf> {
        match self {
            Command::Query(leaf) => Some(leaf.as_ref()),
            Command::Compare(leaf) => Some(leaf.as_ref()),
            Command::Sites(command) => Some(command.leaf()),
            Command::Sitemaps(command) => Some(command.leaf()),
            Command::Inspect(leaf) => Some(leaf),
            Command::Auth(command) => Some(command.leaf()),
            Command::Commands => None,
        }
    }
}

/// The clap command, with each leaf's `--help` footer generated from its spec
/// (decision 13), so the help cannot drift from the code.
#[must_use]
pub fn command() -> clap::Command {
    let mut cmd = Cli::command();
    for (path, spec) in catalog::registry() {
        with_leaf_help(&mut cmd, path, catalog::leaf_help(spec));
    }
    cmd
}

fn with_leaf_help(cmd: &mut clap::Command, path: &[&str], help: String) {
    match path.split_first() {
        None => *cmd = std::mem::take(cmd).after_help(help),
        Some((name, rest)) => {
            if let Some(sub) = cmd.find_subcommand_mut(name) {
                with_leaf_help(sub, rest, help);
            }
        }
    }
}

/// Parses the command line and runs the one command it names.
///
/// clap exits by itself on `--help`, `--version` and a usage error.
///
/// # Errors
///
/// Any error of decision 5. `main` prints it and exits with its code.
pub fn run() -> Result<ExitStatus, Error> {
    let matches = command().get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());
    let format = cli.global.format;
    let Some(leaf) = cli.command.leaf() else {
        if format == Format::Csv {
            return Err(csv_error());
        }
        output::print_response(&catalog::catalog(VERSION), Some("commands"), format);
        return Ok(ExitStatus::Success);
    };
    if format == Format::Csv && !leaf.spec().tabular {
        return Err(csv_error());
    }
    let mut ctx = cli.global.context()?;
    leaf.run(&mut ctx)?;
    Ok(ExitStatus::Success)
}

fn csv_error() -> Error {
    Error::config("--format csv works with query and compare only: use json or jsonl")
}

impl Global {
    fn context(self) -> Result<Context, Error> {
        let base_url = parse_base_url(&self.base_url)?;
        let http = Http::new(
            Duration::from_secs(self.timeout),
            self.retries,
            self.verbose,
            &format!("gsc-cli/{VERSION}"),
        );
        let auth = AuthConfig {
            access_token: self.access_token,
            credentials: self.credentials,
            env: Env::from_process(),
        };
        Ok(Context {
            api: Api::new(base_url),
            format: self.format,
            dry_run: self.dry_run,
            client: Client::new(http, auth),
        })
    }
}
