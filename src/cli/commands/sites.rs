//! `sites`: the properties the credentials can read.

use crate::api::{SITES_GET, SITES_LIST};
use crate::cli::commands::{Context, Leaf, SiteArgs};
use crate::cli::spec::CommandSpec;
use crate::error::Error;
use clap::{Args, Subcommand};

#[derive(Subcommand, Debug)]
pub(crate) enum SitesCommand {
    /// Every property the credentials can read, with the permission level on
    /// each. The names are the exact values for --site.
    List(List),
    /// One property and the permission level on it.
    Get(Get),
}

impl SitesCommand {
    pub(crate) fn leaf(&self) -> &dyn Leaf {
        match self {
            SitesCommand::List(leaf) => leaf,
            SitesCommand::Get(leaf) => leaf,
        }
    }
}

pub(crate) static LIST: CommandSpec = CommandSpec {
    summary: "List the properties the credentials can read",
    endpoints: &[SITES_LIST],
    token_endpoint: false,
    tabular: false,
    list_key: Some("siteEntry"),
    dry_run: true,
    example: "gsc-cli sites list",
};

pub(crate) static GET: CommandSpec = CommandSpec {
    summary: "Get one property and the permission level on it",
    endpoints: &[SITES_GET],
    token_endpoint: false,
    tabular: false,
    list_key: None,
    dry_run: true,
    example: "gsc-cli sites get --site sc-domain:example.com",
};

#[derive(Args, Debug)]
pub(crate) struct List {}

impl Leaf for List {
    fn spec(&self) -> &'static CommandSpec {
        &LIST
    }

    fn run(&self, ctx: &mut Context) -> Result<(), Error> {
        let request = ctx.api.sites_list()?;
        ctx.single(&LIST, &request)
    }
}

#[derive(Args, Debug)]
pub(crate) struct Get {
    #[command(flatten)]
    site: SiteArgs,
}

impl Leaf for Get {
    fn spec(&self) -> &'static CommandSpec {
        &GET
    }

    fn run(&self, ctx: &mut Context) -> Result<(), Error> {
        let request = ctx.api.sites_get(&self.site.site)?;
        ctx.single(&GET, &request)
    }
}
