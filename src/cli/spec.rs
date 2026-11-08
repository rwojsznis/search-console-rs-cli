//! What the catalog and the help footer know about each command
//! (decision 13).

use crate::api::Endpoint;

#[derive(Debug)]
pub struct CommandSpec {
    pub summary: &'static str,
    /// The Search Console endpoints it calls, in order.
    pub endpoints: &'static [Endpoint],
    /// It calls the OAuth token endpoint itself (the `auth` commands). Every
    /// other command does too, unless it is given an access token.
    pub token_endpoint: bool,
    /// `--format csv` works: the output is rows.
    pub tabular: bool,
    /// The array `--format jsonl` prints one line per item of.
    pub list_key: Option<&'static str>,
    /// `--dry-run` prints the requests without reading credentials.
    pub dry_run: bool,
    pub example: &'static str,
}
