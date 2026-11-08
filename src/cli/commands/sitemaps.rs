//! `sitemaps`: the sitemaps submitted for a property, with their counts,
//! errors and warnings.

use crate::api::{SITEMAPS_GET, SITEMAPS_LIST};
use crate::cli::commands::{Context, Leaf, SiteArgs};
use crate::cli::spec::CommandSpec;
use crate::error::Error;
use crate::site::parse_page_url;
use clap::{Args, Subcommand};

#[derive(Subcommand, Debug)]
pub(crate) enum SitemapsCommand {
    /// The sitemaps submitted for the property (or listed in one sitemap
    /// index), with errors, warnings and submitted/indexed counts.
    List(List),
    /// One sitemap.
    Get(Get),
}

impl SitemapsCommand {
    pub(crate) fn leaf(&self) -> &dyn Leaf {
        match self {
            SitemapsCommand::List(leaf) => leaf,
            SitemapsCommand::Get(leaf) => leaf,
        }
    }
}

pub(crate) static LIST: CommandSpec = CommandSpec {
    summary: "List the sitemaps submitted for a property",
    endpoints: &[SITEMAPS_LIST],
    token_endpoint: false,
    tabular: false,
    list_key: Some("sitemap"),
    dry_run: true,
    example: "gsc-cli sitemaps list --site sc-domain:example.com",
};

pub(crate) static GET: CommandSpec = CommandSpec {
    summary: "Get one sitemap: errors, warnings, submitted and indexed counts",
    endpoints: &[SITEMAPS_GET],
    token_endpoint: false,
    tabular: false,
    list_key: None,
    dry_run: true,
    example: "gsc-cli sitemaps get --site sc-domain:example.com https://example.com/sitemap.xml",
};

#[derive(Args, Debug)]
pub(crate) struct List {
    #[command(flatten)]
    site: SiteArgs,

    /// Only the sitemaps listed in this sitemap index (sends sitemapIndex).
    #[arg(long, value_parser = parse_page_url, value_name = "URL")]
    sitemap_index: Option<String>,
}

impl Leaf for List {
    fn spec(&self) -> &'static CommandSpec {
        &LIST
    }

    fn run(&self, ctx: &mut Context) -> Result<(), Error> {
        let request = ctx
            .api
            .sitemaps_list(&self.site.site, self.sitemap_index.as_deref())?;
        ctx.single(&LIST, &request)
    }
}

#[derive(Args, Debug)]
pub(crate) struct Get {
    #[command(flatten)]
    site: SiteArgs,

    /// The sitemap's URL (the feedpath), as `sitemaps list` prints it.
    #[arg(value_parser = parse_page_url, value_name = "FEEDPATH")]
    feedpath: String,
}

impl Leaf for Get {
    fn spec(&self) -> &'static CommandSpec {
        &GET
    }

    fn run(&self, ctx: &mut Context) -> Result<(), Error> {
        let request = ctx.api.sitemaps_get(&self.site.site, &self.feedpath)?;
        ctx.single(&GET, &request)
    }
}
