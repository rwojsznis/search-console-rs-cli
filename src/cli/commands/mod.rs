//! One module per command group. Each leaf has a clap args struct, a
//! `static` spec, and a `Leaf` impl (decision 13).

pub(crate) mod auth;
pub(crate) mod compare;
pub(crate) mod inspect;
pub(crate) mod query;
pub(crate) mod sitemaps;
pub(crate) mod sites;

use crate::api::{Api, ApiRequest};
use crate::cli::spec::CommandSpec;
use crate::client::Client;
use crate::error::Error;
use crate::output::{self, Format};
use crate::site::Site;
use clap::Args;
use serde_json::{Value, json};

pub(crate) trait Leaf {
    fn spec(&self) -> &'static CommandSpec;

    /// Validates, then prints the requests (`--dry-run`) or sends them and
    /// prints the result.
    fn run(&self, ctx: &mut Context) -> Result<(), Error>;
}

/// What every leaf runs with.
#[derive(Debug)]
pub(crate) struct Context {
    pub(crate) api: Api,
    pub(crate) format: Format,
    pub(crate) dry_run: bool,
    pub(crate) client: Client,
}

impl Context {
    pub(crate) fn send(&mut self, request: &ApiRequest) -> Result<Value, Error> {
        self.client.send(request)
    }

    /// `--dry-run`: every request the command would send, in order
    /// (decision 7). json prints `{"requests": [...]}`; jsonl and csv print
    /// one request per line.
    pub(crate) fn print_dry_run(&self, requests: &[ApiRequest]) {
        let requests: Vec<Value> = requests.iter().map(ApiRequest::to_json).collect();
        let format = match self.format {
            Format::Json => Format::Json,
            Format::Jsonl | Format::Csv => Format::Jsonl,
        };
        output::print_response(&json!({"requests": requests}), Some("requests"), format);
    }

    /// The whole run of a one-request command: the dry run, or the response
    /// as Google sent it (decision 3).
    pub(crate) fn single(&mut self, spec: &CommandSpec, request: &ApiRequest) -> Result<(), Error> {
        if self.dry_run {
            self.print_dry_run(std::slice::from_ref(request));
            return Ok(());
        }
        let value = self.send(request)?;
        output::print_response(&value, spec.list_key, self.format);
        Ok(())
    }
}

/// `--site`, for every command about one property.
#[derive(Args, Debug)]
pub(crate) struct SiteArgs {
    /// The property: `sc-domain:example.com` (domain) or `https://example.com/`
    /// (URL prefix), as `gsc-cli sites list` prints it.
    #[arg(long, short = 's', env = "GSC_SITE", hide_env_values = true, value_parser = Site::parse, value_name = "SITE")]
    pub(crate) site: Site,
}
