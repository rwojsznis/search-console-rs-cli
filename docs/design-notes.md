# Design notes

Why `gsc-cli` looks the way it does. Source comments refer to the numbered
decisions here ("decision 5"), so keep the numbering stable.

Google's [Search Console API reference](https://developers.google.com/webmaster-tools/v1/api_reference_index)
is the specification.

---

## Decisions

### 1. Read-only by construction

Two independent guarantees:

- **The code.** `ApiRequest` has no public constructor. The only way to get
  one is a method of `Api`, one per endpoint in `api::ENDPOINTS`: list and
  get sites, list and get sitemaps, Search Analytics query, URL Inspection.
  Every one of them reads. The two POSTs are queries that Google sends as
  POST because their parameters do not fit in a URL. `http::Method` has
  `Get` and `Post` only. A catalog test asserts that every command's
  endpoints are in `ENDPOINTS`, and a unit test pins the two POST paths.
- **The scope.** The CLI asks Google for
  `https://www.googleapis.com/auth/webmasters.readonly` only (service
  account JWTs and `auth login`). With that scope, Google refuses a write
  even if the code sent one. (An access token or a gcloud file brings
  whatever scope it was made with.)

So an agent can run any command without a confirmation step. `README.md`
and the top-level `--help` say so. `auth login` writes one local file, the
credentials, and nothing else.

### 2. A small hand-written client, no Google client crate

The read-only surface is six endpoints. We send them with
[ureq](https://crates.io/crates/ureq) 3 (blocking, rustls) and keep the
JSON as `serde_json::Value`.

- **`google-searchconsole1`** (google-apis-rs) is complete, but generated:
  hyper and a second TLS stack, every field an `Option`, verbose builders,
  and its own error types between us and Google's error body.
- **`google-search-console-api`** (uiuifree) is small, but lags the API: no
  `hour` dimension, no `hourly_all`, no response metadata, and it still
  wraps the Mobile-Friendly Test API that Google shut down in 2023.
- **Typed responses lose data.** serde drops fields Google adds. We print
  Google's JSON unchanged where we can (decision 3), so a typed model would
  only be a way to lose fields.

No async runtime: nothing runs concurrently (decision 10), so blocking
ureq is simpler than tokio and has fewer dependencies.

### 3. What stdout holds

- **`sites`, `sitemaps`, `inspect`:** Google's JSON, unchanged.
  `serde_json`'s `preserve_order` keeps Google's key order. No envelope, no
  renamed keys. Agents know Google's documented shapes.
- **`query` and `compare`:** rows with **named keys**. Google's rows are
  `{"keys": ["https://…", "shoes"], "clicks": …}`: the meaning of each key
  depends on the order of `dimensions` in the request, which an agent
  reading the output no longer has. So each row becomes
  `{"page": "https://…", "query": "shoes", "clicks": …, "impressions": …,
  "ctr": …, "position": …}`. Metric values are Google's, unchanged.
- **json** wraps the rows in an envelope with the resolved request (the
  dates the probe chose, decision 9), `responseAggregationType`,
  `metadata` (where incomplete data starts), `rowCount` and `truncated`.
  `compare` adds `current`, `previous` and `totals`.
- Keys are camelCase everywhere, as in Google's API: `searchAppearance`,
  `clicksChange`.

### 4. Three output formats, no TTY tricks

`--format json` (default) is pretty JSON with a 2-space indent.
`--format jsonl` is one compact value per line: one row (`query`,
`compare`), one list item (`sites list`, `sitemaps list`, `commands`), or
the whole body. `--format csv` is a header line and one line per row, and
works with `query` and `compare` only; elsewhere it is a `config` error.
CSV gives an agent the same rows in far fewer tokens than JSON.

No colors, no tables, no spinners, no prompts. The bytes are the same on a
terminal and in a pipe.

### 5. Errors and exit codes

| Exit | Meaning | `error.kind` |
|---|---|---|
| 0 | Success. Also when stdout closed early (broken pipe). | — |
| 1 | Search Console answered with a non-2xx status (not 401) | `api` |
| 2 | Usage or configuration error: clap usage errors, a bad date, a bad base URL, `--format csv` where it does not apply | `config` (clap usage errors keep clap's own text) |
| 3 | No usable answer: connect, TLS, timeout, a body that is not JSON | `transport`, `timeout`, `decode` |
| 4 | No credentials, unreadable credentials, the token endpoint refused, or the API answered 401 | `auth` |

`auth` has its own code because an agent must act differently: a person
has to log in, make a key, or grant a scope. Retrying will not help, and
neither will changing the request.

An error prints exactly one JSON line on stderr and nothing on stdout:

```json
{"error":{"kind":"api","message":"Search Console returned 403: User does not have sufficient permission for site 'x'.","hint":"Pass --site exactly as `gsc-cli sites list` prints it: …","status":403,"reason":"PERMISSION_DENIED","body":{…},"request":{"method":"GET","url":"https://searchconsole.googleapis.com/webmasters/v3/sites/x"}}}
```

- `status`, `reason` and `body` are there when a server answered. `reason`
  is Google's `error.status` (or `errors[0].reason`), or the OAuth `error`
  code (`invalid_grant`) from the token endpoint. `body` is the parsed
  JSON, or the text cut to 4 KiB.
- `request` is the first request the error belongs to. A token error keeps
  the token endpoint, not the API request that needed the token.
- `hint` is there for the errors an agent can act on: no credentials, a
  refused token, a missing scope, a property name that does not match, the
  API not enabled in the Cloud project, quota. The table is in
  `src/error.rs`.
- Warnings that do not fail the run are one JSON line each on stderr:
  `{"warning":{"kind","message"}}`. Kinds: `truncated`, `row_cap`,
  `no_recent_data`, `before_retention`, `retry`, `scope`, `browser`.

A closed stdout pipe is not an error (`src/cli/out.rs`, ported from
postmark-rs-cli), and it stops paging (decision 10).

### 6. Credentials

The first source that is set wins:

1. `--access-token` / `GSC_ACCESS_TOKEN`: used as given (for example
   `gcloud auth print-access-token`). Must be a valid header value.
2. `--credentials` / `GSC_CREDENTIALS`: a key file.
3. `<config dir>/credentials.json`, written by `auth login`. The config dir
   is `GSC_CONFIG_DIR`, else `$XDG_CONFIG_HOME/gsc-cli`, else
   `~/.config/gsc-cli`.
4. `GOOGLE_APPLICATION_CREDENTIALS`.
5. gcloud's application default credentials:
   `$CLOUDSDK_CONFIG/application_default_credentials.json`, else
   `~/.config/gcloud/application_default_credentials.json`.

Our own login file comes before `GOOGLE_APPLICATION_CREDENTIALS` on
purpose: that variable is often set for other Google Cloud work, to a
service account that cannot read Search Console. A source that is set but
cannot be read is an `auth` error; the CLI never falls through to the next
source, because that would use credentials the user did not choose.

Key files are Google's own formats:

- `service_account`: the CLI signs an RS256 JWT (`iss`, `scope`, `aud`,
  `iat`, `exp`) and trades it at `token_uri` for an access token. The
  signing uses `ring`, which rustls already brings in: a JWT crate would add
  a second RSA implementation, and the `rsa` crate carries an advisory that
  `cargo audit` fails on. PKCS#8 and PKCS#1 PEM keys work.
- `authorized_user`: the refresh token grant at `token_uri`. gcloud and
  `auth login` write this format.

Other types (`external_account`, impersonation) are an `auth` error that
names the two supported types.

The credentials are read, and the token fetched, on the first request, once
per run. `--dry-run` and a command that fails validation never touch them.
There is no token cache on disk: one token request per run is cheap, and a
cache would be one more secret on disk.

**Secrets never appear** in stdout, stderr, `--verbose`, `--dry-run` or an
error: not the access token, refresh token, client secret or private key.
The types that hold them have a hand-written `Debug`. The tests run every
path with sentinel secrets and assert they are absent.

### 7. Global flags

All use `global = true`, so they work before or after the subcommand.

| Flag | Env | Default | What |
|---|---|---|---|
| `--access-token` | `GSC_ACCESS_TOKEN` | — | decision 6 |
| `--credentials` | `GSC_CREDENTIALS` | — | decision 6 |
| `--base-url` | `GSC_BASE_URL` | `https://searchconsole.googleapis.com/` | Tests point it at a mock. http or https; a trailing `/` is added. |
| `--timeout <SECONDS>` | — | `60` | Per request, ureq's global timeout |
| `--retries <N>` | — | `2` | decision 12 |
| `--format` | — | `json` | decision 4 |
| `--dry-run` | — | off | Prints `{"requests": [{method, url, body}]}`: every request the command would send, in order, without credentials or network. Values that depend on an earlier answer are placeholders such as `"<latest day with data - 27 days>"`. Paged commands show the first page. |
| `-v`, `--verbose` | — | off | One JSON line on stderr before each request (`{"request":{method,url}}`) and one after (`{"response":{status,ms}}`), token requests included. |

`--site` / `GSC_SITE` belongs to the commands about one property.

### 8. Command tree and naming

| Command | Endpoint |
|---|---|
| `query` | `POST /webmasters/v3/sites/{siteUrl}/searchAnalytics/query` (probe, then pages) |
| `compare` | the same, 2 to 5 times (decision 11) |
| `sites list` / `sites get` | `GET /webmasters/v3/sites[/{siteUrl}]` |
| `sitemaps list` / `sitemaps get <FEEDPATH>` | `GET /webmasters/v3/sites/{siteUrl}/sitemaps[/{feedpath}]` |
| `inspect <URL>` | `POST /v1/urlInspection/index:inspect` |
| `auth login`, `auth status` | the OAuth token endpoint; `status` also lists sites |
| `commands` | none (decision 13) |

- **Flag names:** the API field, split into words, hyphenated, lowercase:
  `startDate` → `--start-date`, `dataState` → `--data-state`,
  `aggregationType` → `--aggregation-type`, `sitemapIndex` →
  `--sitemap-index`, `languageCode` → `--language-code`. Exceptions: `type`
  is `--type`; `siteUrl` is `--site`; `dimensionFilterGroups` is
  `--filter`; paging is `--limit`/`--all` (decision 10).
- **Enum values** are the exact API spelling (`searchAppearance`,
  `hourly_all`, `byPage`, `googleNews`), accepted with `ignore_case`, and
  sent exactly. A unit test checks that each enum's `as_str` equals clap's
  name.
- **`--filter DIMENSION:OPERATOR:EXPRESSION`**, split on the first two
  colons only, so a URL or a regex keeps its own colons. Repeat it for AND
  (Google supports `groupType: "and"` only); use a regex with `|` for OR.
- **The site** is `sc-domain:example.com` or an http(s) URL. A bare
  `example.com` could be either property type, so it is rejected with both
  spellings rather than guessed. URLs are normalized the way `Url`
  serializes them (lowercase host, `/` for an empty path), because the API
  matches the name exactly. The site is one path segment, encoded like
  `encodeURIComponent`.

### 9. Dates

Search Console's days are Pacific Time days. The CLI sends `YYYY-MM-DD` and
nothing else.

- `--start-date`, `--end-date`, `--days N` (default 28). `--days` counts
  back from `--end-date`, or forward from `--start-date`; with both dates it
  is a `config` error.
- **Without `--end-date`, the range ends on the latest day with data.**
  Final data is 2-3 days behind, and the lag varies. A "last 7 days" that
  ends today has 2-3 empty days, and a comparison of it with the 7 days
  before shows a false drop of 30-40%. So the CLI first sends a
  **probe**: daily totals for the 16 days before today (UTC), with the same
  search type and data state (`all` for `hourly_all`, which only groups by
  hour). The latest date in the answer is the end. If the probe has no
  rows, the end is 3 days ago, with a `no_recent_data` warning. The
  Search Console UI's "Last 28 days" works the same way.
- Google keeps 16 months. A start before that is not an error (Google
  returns no rows for those days), but it gets a `before_retention`
  warning.

### 10. Paging and row limits

- One request asks for at most 25,000 rows (`rowLimit`), and `startRow`
  moves through the pages, one request at a time, never in parallel.
- `--limit N` (default 1000 for `query`, 100 for `compare`) asks for N + 1
  rows, so that the CLI can tell whether rows were left out. Then it says
  so: `truncated: true` in the json envelope and a `truncated` warning.
- `--all` fetches every page until a short page. Google itself keeps at most
  50,000 rows a day per search type.
- `--sort` on `query` (any column; Google sorts by clicks only) fetches
  every row first, up to `--max-rows` (default 100,000), then sorts, then
  applies `--limit`. Hitting `--max-rows` is a `row_cap` warning. A
  missing or `null` value sorts last in both orders.
- jsonl and csv **stream**: each page is printed when it arrives, and a
  closed stdout stops the fetch. json prints once, at the end.

### 11. Compare

`compare` answers "what changed": the agent question that the API cannot
answer in one request.

- The previous period is the one of the same length just before
  (default), the same weekdays 52 weeks earlier (`--yoy`; 364 days, so a
  Monday compares with a Monday), or the same length from
  `--previous-start-date`.
- Requests: the probe when needed, the totals of both periods (no
  dimensions), then every row of both periods (up to `--max-rows`).
  Without dimensions, the totals are the one row.
- **Both periods are fetched in full before the join.** With a top-N
  fetch, a page that fell out of the top N would look like it lost all its
  clicks.
- Rows join on their keys. A row in one period only has 0 clicks and
  impressions in the other, and `null` CTR and position, whose change is
  then `null` too.
- **Totals come from their own requests**, so they include the anonymized
  queries that the rows leave out. The sum of the rows is lower than the
  totals, by design of the API.
- Columns: each metric, `…Previous` and `…Change`. A negative
  `positionChange` is an improvement. Computed numbers are rounded to 6
  decimals; whole ones print as integers.

### 12. Retries

A 429, 500, 502, 503 or 504 is retried up to `--retries` times (default 2),
after `Retry-After` (at most 60 s) or 0.5 s, 1 s, 2 s… Each retry is a
`retry` warning. Transport failures and timeouts are not retried: a
timeout retried twice would triple the wait.

### 13. Agents can discover every command

`gsc-cli commands` prints the catalog as JSON: per command its `summary`,
`requests` (method and path template), `formats`, `list_key`, `example`
and `args`. `args` come from clap's own `Command` introspection, so they
cannot drift from the parser. Each leaf's `--help` ends with `Requests:`,
`Formats:` and `Example:`, generated from the same `CommandSpec`.

One module per command group in `src/cli/commands/`. Each leaf has a clap
args struct, a `static` `CommandSpec`, and a `Leaf` impl. The registry in
`src/cli/catalog.rs` maps each leaf's path to its spec. `docs/commands.md`
is generated from the catalog by a test.

### 14. Tests

- **Unit tests** next to the code: request building, encoding, dates,
  paging, the join, JWT signing, error mapping.
- **Integration tests** in `tests/` run the real binary
  (`env!("CARGO_BIN_EXE_gsc-cli")`) against an `httptest` mock through
  `--base-url`, and a mock token endpoint through `token_uri` in the key
  file. They assert the method, path, exact JSON body and the bearer token.
- **Environment:** the harness removes every variable the CLI reads
  (`GSC_*`, `GOOGLE_APPLICATION_CREDENTIALS`, `CLOUDSDK_CONFIG`,
  `XDG_CONFIG_HOME`, `BROWSER`, proxies) one by one, and points `HOME` at an
  empty directory, so a developer's own credentials are never used.
  **Never `env_clear()`**: it removes `LLVM_PROFILE_FILE`, and
  cargo-llvm-cov then loses the coverage of the spawned binary.
- **`auth login`** is tested end to end: the test reads the consent URL
  from stderr, connects to the loopback redirect with a code and state, and
  checks the PKCE verifier against the challenge at the mock token
  endpoint.
- `tests/fixtures/test-key-*.pem` is a throwaway RSA key for the JWT tests.
- **Live tests** (`tests/live.rs`): every test is `#[ignore]` and reads
  `GSC_LIVE_SITE` with the developer's own credentials. Never run in CI.
  Safe because of decision 1.

### 15. mise manages the toolchain and every dev tool

The same setup as postmark-rs-cli: **one pin per tool, read the same way
locally and in CI.**

- Rust is pinned in `rust-toolchain.toml` only (`1.99.1`, components
  `rustfmt`, `clippy`, `llvm-tools`); `mise.toml` turns on
  `idiomatic_version_file_enable_tools = ["rust"]` so mise installs it from
  there. No `rust = "…"` in `mise.toml`: mise exports `RUSTUP_TOOLCHAIN`,
  which would override the file.
- Dev tools are pinned in `mise.toml` `[tools]` with prebuilt-binary
  backends: cargo-llvm-cov, cargo-machete, cargo-audit, actionlint.
- `mise.toml` tasks are the single list of check commands: `fmt`,
  `clippy`, `doc`, `machete`, `audit`, `lint`, `test`, `test-release`,
  `test-doc`, `coverage`, `coverage-lcov`, `actionlint`, `check`. AGENTS.md
  and CI call them and never repeat the command lines.
- CI installs everything with `jdx/mise-action`, pinned by digest.
  Renovate bumps the toolchain, `rust-version`, the tools and the actions.

### 16. `auth login`

Google's OAuth flow for installed apps, with a loopback redirect and PKCE
(RFC 7636, S256):

- The user brings the OAuth client: the JSON of a **Desktop app** client
  from the Google Cloud console (`--client-secret` / `GSC_CLIENT_SECRET`).
  The CLI ships no client of its own: a shared client would need Google's
  app verification, and its quota would be everyone's.
- The CLI listens on `127.0.0.1:<random port>`, prints the consent URL as
  a JSON line on stderr, and starts `$BROWSER` or the platform opener
  (`open`, `xdg-open`) unless `--no-browser`. It asks for
  `webmasters.readonly` only, with `access_type=offline` and
  `prompt=consent` so that Google always sends a refresh token.
- It answers requests that are not the redirect (`/favicon.ico`) with 404
  and keeps waiting, up to `--wait` seconds. A wrong `state`, an `error=`
  redirect, an answer without a refresh token, or a grant without the
  Search Console scope (Google's consent screen lets the user untick it)
  is an `auth` error.
- It writes an `authorized_user` file (gcloud's format) to
  `<config dir>/credentials.json`, mode 0600 in a 0700 directory, through a
  temporary file and a rename.

People who already use gcloud can skip it:
`gcloud auth application-default login --client-id-file=client_secret.json
--scopes=https://www.googleapis.com/auth/webmasters.readonly` writes the
same format to the place decision 6 looks.
