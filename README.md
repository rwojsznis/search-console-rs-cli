# gsc-cli
[![CI](https://github.com/rwojsznis/search-console-rs-cli/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/rwojsznis/search-console-rs-cli/actions/workflows/ci.yml?query=branch%3Amain)
[![codecov](https://codecov.io/gh/rwojsznis/search-console-rs-cli/graph/badge.svg)](https://codecov.io/gh/rwojsznis/search-console-rs-cli)

A command-line client for the [Google Search Console API](https://developers.google.com/webmaster-tools/v1/api_reference_index),
built for LLM agents and for people who look into search traffic: clicks,
impressions, CTR and position by page, query, date, country, device or
search appearance, with regex filters; period comparisons; sitemaps; and URL
Inspection.

> [!NOTE]
> **Unofficial.** This project is not affiliated with Google.

**Read-only.** `gsc-cli` asks Google for the `webmasters.readonly` scope
only, and calls only endpoints that read. No command changes anything in
Search Console, so an agent can run any command without asking first.

## Install

Each [release](https://github.com/rwojsznis/search-console-rs-cli/releases/latest)
has one archive per platform, and a `SHA256SUMS` file:

| Platform | Archive |
|---|---|
| macOS, Apple silicon | `gsc-cli-aarch64-apple-darwin.tar.gz` |
| macOS, Intel | `gsc-cli-x86_64-apple-darwin.tar.gz` |
| Linux x86_64, static (any distribution) | `gsc-cli-x86_64-unknown-linux-musl.tar.gz` |
| Linux x86_64, glibc | `gsc-cli-x86_64-unknown-linux-gnu.tar.gz` |

```bash
TARGET=aarch64-apple-darwin   # one of the targets in the table above
BASE=https://github.com/rwojsznis/search-console-rs-cli/releases/latest/download
curl -fLO "$BASE/gsc-cli-$TARGET.tar.gz"
curl -fLO "$BASE/SHA256SUMS"

# Linux: sha256sum --check --ignore-missing SHA256SUMS
shasum -a 256 --check --ignore-missing SHA256SUMS

tar -xzf "gsc-cli-$TARGET.tar.gz"
mkdir -p ~/.local/bin && mv gsc-cli ~/.local/bin/
gsc-cli --version
```

The macOS binaries are not signed. If you download with a browser, remove
the quarantine flag first: `xattr -d com.apple.quarantine gsc-cli`.

Or build it from source (`--version` then shows `0.0.0`):

```bash
cargo install --git https://github.com/rwojsznis/search-console-rs-cli --locked
```

## Credentials

Every way needs a Google Cloud project with the **Google Search Console
API** enabled (APIs & Services, Library). Then pick one:

**A service account** (best for agents and servers: no browser, no
expiry). Create a service account and a JSON key in the Cloud console. In
Search Console, add the service account's email as a user of each property
(Settings, Users and permissions; "Restricted" is enough). Then:

```bash
export GSC_CREDENTIALS=~/keys/gsc-reader.json
```

**Your own Google account, with a browser.** Create an OAuth client of type
**Desktop app** (APIs & Services, Credentials), download its JSON, and run:

```bash
gsc-cli auth login --client-secret ~/Downloads/client_secret_….json
```

It asks for read-only access and saves the credentials to
`~/.config/gsc-cli/credentials.json` (mode 0600). With gcloud, this does the
same:
`gcloud auth application-default login --client-id-file=client_secret.json --scopes=https://www.googleapis.com/auth/webmasters.readonly`.

**An access token** you already have: `export GSC_ACCESS_TOKEN=…`.

The first one set wins, in this order: `GSC_ACCESS_TOKEN`,
`GSC_CREDENTIALS`, the `auth login` file, `GOOGLE_APPLICATION_CREDENTIALS`,
gcloud's application default credentials. Check the setup end to end:

```bash
gsc-cli auth status
```

## Quick start

```bash
gsc-cli sites list                      # the exact property names
export GSC_SITE=sc-domain:example.com   # or https://example.com/ for a URL-prefix property

# Totals for the last 28 days with data.
gsc-cli query

# Top pages, and top queries of the blog, by clicks.
gsc-cli query --dimensions page --limit 20
gsc-cli query -d query --filter 'page:includingRegex:^https://example\.com/blog/' --limit 50

# Every query of one page, as CSV.
gsc-cli query -d query --filter page:equals:https://example.com/pricing --all --format csv

# Many impressions, few clicks: titles and snippets to fix.
gsc-cli query -d page --sort impressions --limit 50 --format csv

# Non-brand traffic by device, day by day.
gsc-cli query -d date,device --filter 'query:excludingRegex:(?i)acme|akme' --days 90

# What dropped this week, page by page?
gsc-cli compare -d page --days 7 --sort clicksChange --order asc --limit 20

# This month against the same weeks last year.
gsc-cli compare -d query --days 28 --yoy

# Why is this page not in Google?
gsc-cli inspect https://example.com/pricing

# Sitemaps: errors, warnings, submitted and indexed counts.
gsc-cli sitemaps list
```

Every command has `--help`, which ends with the requests it sends and an
example. [`docs/commands.md`](docs/commands.md) lists them all.

## What the data is, and is not

- **Metrics:** clicks, impressions, CTR (a fraction: `0.034` is 3.4%) and
  average position, for Google Search (`--type web`, `image`, `video`,
  `news`), Discover and Google News.
- **Dimensions:** `date`, `hour` (with `--data-state hourly_all`), `query`,
  `page`, `country`, `device`, `searchAppearance`, in any combination.
- **Filters:** `equals`, `notEquals`, `contains`, `notContains`,
  `includingRegex`, `excludingRegex` (RE2) on `query`, `page`, `country`,
  `device`, `searchAppearance`. Repeat `--filter` for AND; use one regex
  with `|` for OR.
- **16 months** of history. Google deletes older days.
- **Final data is 2-3 days behind.** Without `--end-date`, ranges end on the
  latest day with data (one extra request finds it), so "the last 7 days"
  never includes empty days. `--data-state all` adds fresh, still-changing
  data.
- **Rows do not add up to the totals.** Google leaves anonymized (rare)
  queries out of query rows, and keeps at most 50,000 rows a day. `compare`
  asks for the totals separately for this reason.
- **Grouping by page changes the totals** (Google counts by page instead of
  by property); see `--aggregation-type`.
- **Not in the API:** the page indexing report, Core Web Vitals, links,
  manual actions and security issues. URL Inspection covers one URL at a
  time (2,000 a day per property).

## Output and exit codes

- **stdout:** `--format json` (default) is pretty JSON; `jsonl` is one
  compact value per line (one row, or one list item); `csv` (for `query` and
  `compare`) is a header and one line per row. `query` and `compare` rows
  have named keys (`{"page": …, "query": …, "clicks": …}`). `sites`,
  `sitemaps` and `inspect` print Google's JSON unchanged.
- **stderr:** one JSON line per error or warning.

  ```json
  {"error":{"kind":"api","message":"Search Console returned 403: User does not have sufficient permission for site …","hint":"Pass --site exactly as `gsc-cli sites list` prints it: …","status":403,"reason":"PERMISSION_DENIED","body":{…},"request":{"method":"GET","url":"…"}}}
  ```

| Exit | Meaning | `error.kind` |
|---|---|---|
| 0 | Success, also when stdout closes early (`\| head`) | — |
| 1 | Search Console answered with an error status | `api` |
| 2 | Usage or configuration error; nothing was sent | `config` (clap's own text for bad flags) |
| 3 | No usable answer: connection, TLS, timeout, or a body that is not JSON | `transport`, `timeout`, `decode` |
| 4 | No credentials, or Google refused them: a person must act | `auth` |

## For AI agents

- **Discover the commands:** `gsc-cli commands` prints a JSON catalog of
  every command: its requests, formats, example and flags (read from the
  parser itself). No network, no credentials.
- **Read-only by construction:** running any command is safe.
- **Use `--format csv`** for `query` and `compare`: the same rows in far
  fewer tokens than JSON.
- **Mind the limits:** `query` prints 1,000 rows by default and `compare`
  100; a `truncated` warning says when more match. Use `--limit`, `--all`,
  or a narrower `--filter`.
- **Check before you send:** `--dry-run` prints every request, without
  credentials or network.
- **Exit 4** means a person must fix the credentials; run
  `gsc-cli auth status` and show them its output.
- **Errors are JSON** on stderr, with a `hint` for the errors you can act
  on (a wrong property name, a missing scope, the API not enabled, quota).

## License

MIT. See [LICENSE.txt](LICENSE.txt).
