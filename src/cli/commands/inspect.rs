//! `inspect`: URL Inspection, Google's view of one URL of a property.

use crate::api::URL_INSPECTION;
use crate::cli::commands::{Context, Leaf, SiteArgs};
use crate::cli::spec::CommandSpec;
use crate::error::Error;
use crate::site::parse_page_url;
use clap::Args;

pub(crate) static SPEC: CommandSpec = CommandSpec {
    summary: "Inspect one URL: index status, canonical, last crawl, rich results",
    endpoints: &[URL_INSPECTION],
    token_endpoint: false,
    tabular: false,
    list_key: None,
    dry_run: true,
    example: "gsc-cli inspect --site sc-domain:example.com https://example.com/pricing",
};

/// Google's index status of one URL: whether it is indexed and why not,
/// the Google-selected canonical, the last crawl, robots.txt and fetch
/// state, mobile usability, AMP and rich results. The quota is 2,000
/// inspections a day and 600 a minute per property.
#[derive(Args, Debug)]
pub(crate) struct Inspect {
    #[command(flatten)]
    site: SiteArgs,

    /// The URL to inspect. It must be in the property (sends inspectionUrl).
    #[arg(value_parser = parse_page_url, value_name = "URL")]
    url: String,

    /// The language of the messages in the result, as a BCP-47 code: en-US,
    /// de (sends languageCode).
    #[arg(long, value_name = "CODE")]
    language_code: Option<String>,
}

impl Leaf for Inspect {
    fn spec(&self) -> &'static CommandSpec {
        &SPEC
    }

    fn run(&self, ctx: &mut Context) -> Result<(), Error> {
        let request = ctx
            .api
            .inspect(&self.site.site, &self.url, self.language_code.as_deref())?;
        ctx.single(&SPEC, &request)
    }
}
