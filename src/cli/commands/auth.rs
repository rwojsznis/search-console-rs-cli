//! `auth`: log in, and check the credentials (decisions 6 and 16).

use crate::api::SITES_LIST;
use crate::auth::login::{self, Options};
use crate::cli::commands::{Context, Leaf};
use crate::cli::spec::CommandSpec;
use crate::error::Error;
use crate::output::{self, warn};
use clap::{Args, Subcommand};
use serde_json::{Map, Value};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Subcommand, Debug)]
pub(crate) enum AuthCommand {
    /// Log in with a browser and save the credentials, for people. Needs
    /// the client secret of a "Desktop app" OAuth client. Asks for the
    /// webmasters.readonly scope only.
    Login(Login),
    /// Show which credentials are used, get an access token, and count the
    /// properties they can read. The first command to run when something
    /// fails with exit code 4.
    Status(Status),
}

impl AuthCommand {
    pub(crate) fn leaf(&self) -> &dyn Leaf {
        match self {
            AuthCommand::Login(leaf) => leaf,
            AuthCommand::Status(leaf) => leaf,
        }
    }
}

pub(crate) static LOGIN: CommandSpec = CommandSpec {
    summary: "Log in with a browser and save the credentials",
    endpoints: &[],
    token_endpoint: true,
    tabular: false,
    list_key: None,
    dry_run: false,
    example: "gsc-cli auth login --client-secret ~/Downloads/client_secret.json",
};

pub(crate) static STATUS: CommandSpec = CommandSpec {
    summary: "Show the credentials in use, and check them end to end",
    endpoints: &[SITES_LIST],
    token_endpoint: true,
    tabular: false,
    list_key: None,
    dry_run: false,
    example: "gsc-cli auth status",
};

#[derive(Args, Debug)]
pub(crate) struct Login {
    /// The client secret JSON of a "Desktop app" OAuth client, from the
    /// Google Cloud console (APIs & Services, Credentials).
    #[arg(long, env = "GSC_CLIENT_SECRET", value_name = "FILE")]
    client_secret: PathBuf,

    /// Only print the URL to open; do not start a browser. $BROWSER, when
    /// set, is the browser to start.
    #[arg(long)]
    no_browser: bool,

    /// Seconds to wait for the browser.
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..=3600), value_name = "SECONDS")]
    wait: u64,
}

impl Leaf for Login {
    fn spec(&self) -> &'static CommandSpec {
        &LOGIN
    }

    fn run(&self, ctx: &mut Context) -> Result<(), Error> {
        if ctx.dry_run {
            return Err(Error::config(
                "auth login has no --dry-run: it waits for a person in a browser",
            ));
        }
        let options = Options {
            client_secret: self.client_secret.clone(),
            open_browser: !self.no_browser,
            browser: std::env::var_os("BROWSER").filter(|value| !value.is_empty()),
            wait: Duration::from_secs(self.wait),
        };
        let result = login::login(ctx.client.http(), &ctx.client.auth().env, &options)?;
        output::print(&result, ctx.format);
        Ok(())
    }
}

#[derive(Args, Debug)]
pub(crate) struct Status {}

impl Leaf for Status {
    fn spec(&self) -> &'static CommandSpec {
        &STATUS
    }

    /// With `--dry-run`, it only reads the credentials: no request.
    fn run(&self, ctx: &mut Context) -> Result<(), Error> {
        let mut status = Map::new();
        status.insert("credentials".into(), ctx.client.credentials()?.describe());
        if ctx.dry_run {
            output::print(&Value::Object(status), ctx.format);
            return Ok(());
        }
        let token = ctx.client.token()?;
        if token.scope.is_some() || token.expires_in.is_some() {
            status.insert("token".into(), token.describe());
        }
        if token.has_scope() == Some(false) {
            warn(
                "scope",
                &format!(
                    "the token's scopes do not include {}, so Search Console will refuse it",
                    crate::auth::SCOPE
                ),
            );
        }
        let sites = ctx.send(&ctx.api.sites_list()?)?;
        let count = sites
            .get("siteEntry")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        status.insert("sites".into(), count.into());
        output::print(&Value::Object(status), ctx.format);
        Ok(())
    }
}
