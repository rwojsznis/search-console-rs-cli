# AGENTS.md

You must use ASD-STE100 Simplified Technical English (STE) when it doesn't detract from meaning.

## What this is

`gsc-cli` is an unofficial, **read-only** command-line client for the
[Google Search Console API](https://developers.google.com/webmaster-tools/v1/api_reference_index).
It is built for LLM agents and for people who look into search traffic:
clicks, impressions, CTR and position by page, query, date, country, device
and search appearance; period comparisons; sitemaps; URL Inspection.
Single crate (`search-console-rs-cli`), one binary (`gsc-cli`) plus a
library (`gsc_cli`).

Read [`docs/design-notes.md`](docs/design-notes.md) before you change
anything non-trivial. It holds the numbered design decisions. Source
comments cite them by number ("decision 5").
[`docs/commands.md`](docs/commands.md) lists every command; a test
generates it from the catalog.

## Sources of truth

- **Google's API reference is the specification**: field names, enum
  spellings, limits. Link the page when a comment depends on it.
- **No Google client crate** (decision 2). We send six endpoints with ureq
  and keep the JSON as `serde_json::Value`.
- **The binary is the product.** Integration tests run the real binary.

## Layout

| Path | What |
|---|---|
| `src/main.rs` | Entry point: run, print the error line, choose the exit code |
| `src/lib.rs` | Module list and the panic-family lints |
| `src/api.rs` | `ENDPOINTS`, `Api` (one method per endpoint), `ApiRequest` (no public constructor), `--base-url` (decisions 1, 8) |
| `src/http.rs` | `Http`: ureq, timeout, retries, `--verbose` lines; `Method` is GET or POST only (decisions 2, 12) |
| `src/client.rs` | `Client`: credentials and token on first use, then `send` |
| `src/auth/mod.rs` | Credential sources and their order, key file parsing, token requests, `Env` (decision 6) |
| `src/auth/jwt.rs` | The service account JWT, signed RS256 with `ring` |
| `src/auth/login.rs` | `auth login`: loopback redirect, PKCE, the private credentials file (decision 16) |
| `src/search.rs` | Search Analytics: enums, `Filter`, `Range`/`Day` and the probe, `fetch` (paging), named rows, `sort`, the compare join (decisions 3, 9, 10, 11) |
| `src/dates.rs` | `YYYY-MM-DD`, day arithmetic, the 16-month retention |
| `src/site.rs` | `Site` (the property name, checked and normalized), `parse_page_url` |
| `src/error.rs` | `Error`, its kinds, exit codes, the JSON error line, the hints (decision 5) |
| `src/output.rs` | `Format`, json/jsonl/csv rendering, warning lines (decision 4) |
| `src/cli/mod.rs` | `Cli`, the global flags, the `Command` enum, the one dispatcher |
| `src/cli/spec.rs` | `CommandSpec`: what the catalog and the help footer know |
| `src/cli/catalog.rs` | The registry (the one list of leaf commands), the `--help` footer, the `commands` catalog (decision 13) |
| `src/cli/commands/mod.rs` | The `Leaf` trait, `Context` (`send`, `single`, `print_dry_run`), `SiteArgs` |
| `src/cli/commands/<group>.rs` | One file per command group: clap args, a `static` spec per leaf, the `Leaf` impl. `query.rs` also holds `SearchArgs` and the helpers `compare.rs` shares |
| `src/cli/exit.rs` | The exit codes, as a type |
| `src/cli/out.rs` | Writes that are safe on a broken pipe, and the print macros |
| `tests/common/mod.rs` | The harness: `cli()`/`run()`/`run_api()`, the empty `HOME`, `expect_get`/`expect_post`/`expect_probe`, sentinels, `assert_no_secret` |
| `tests/query.rs`, `tests/compare.rs` | Search Analytics through the binary |
| `tests/resources.rs` | `sites`, `sitemaps`, `inspect` |
| `tests/auth.rs` | Every credential source, the token endpoint, `auth status`, `auth login` end to end |
| `tests/errors.rs` | Every error path |
| `tests/catalog.rs` | Registry ↔ clap tree ↔ `ENDPOINTS`, every example as a `--dry-run`, help footers, `commands`, the `docs/commands.md` generator |
| `tests/live.rs` | Opt-in smoke tests against the real API (`#[ignore]`; `GSC_LIVE_SITE`, optional `GSC_LIVE_URL`) |
| `tests/fixtures/` | A throwaway RSA key for the JWT tests |

Unit tests live in `#[cfg(test)] mod tests` next to the code; cross-cutting
and CLI-level tests live in `tests/`.

## Workflow — do it in this order

1. **Write the failing test first.** Before any implementation, add a test
   that fails for the reason you expect. Run it and *read the failure* — a
   test that passes immediately, or fails with the wrong message, is not yet
   a test of the thing you meant.
2. **Then write the implementation**, and only enough of it to make that
   test pass.
3. **Check your work.** Run the full suite, not just the new test. If the
   change touches a command, also run the binary and look at the output
   (`--dry-run` needs no credentials); `cargo test` passing is necessary,
   not sufficient.
4. **Run the linters last**, and get them clean before you call the work
   done.

```bash
mise run check      # fmt --check, clippy -D warnings, doc -D warnings, machete, audit, tests
mise run coverage   # cargo llvm-cov summary; new code must be covered
```

These are the same mise tasks that CI runs. `mise.toml` is the only place
that holds the command lines; do not copy them elsewhere.

Keep successful test output brief. We care about failures: if the quiet test
run fails, inspect and report the complete failure output.

Do not report a change as finished until every one of them is green. If
something is left failing or unfinished, say so explicitly rather than
narrowing the scope quietly.

## Coverage

- `mise run coverage` prints the totals and **fails under 99% line
  coverage**; CI's `coverage-lcov` task has the same floor. The floor only
  goes up: when coverage rises, raise `--fail-under-lines` in `mise.toml`
  to the new figure rounded down. Never lower it to make a change pass.
  `mise exec -- cargo llvm-cov --all-targets --show-missing-lines
  --summary-only` lists the uncovered lines.
- Coverage includes the `gsc-cli` processes that the integration tests
  spawn: cargo-llvm-cov instruments the binary and sets `LLVM_PROFILE_FILE`
  for it. That is why `env_clear()` is forbidden in tests (see Gotchas).
- CI uploads `mise run coverage-lcov` to Codecov.

## Toolchain

mise manages Rust and every dev tool (decision 15).

- Run `mise trust` once in a new checkout, then `mise install`.
- Run cargo through mise's environment: `mise exec -- cargo …`, or a
  `mise run` task.
- One pin per tool. Rust is pinned in `rust-toolchain.toml` only; mise reads
  it. Do not add `rust = "…"` to `mise.toml`: mise exports
  `RUSTUP_TOOLCHAIN`, which overrides the file, so two pins could silently
  disagree. The dev tools are pinned in `mise.toml` `[tools]`.
- Never install a dev tool with `cargo install` or Homebrew. If a tool is
  missing, add it to `mise.toml`, with a backend that downloads a prebuilt
  binary (`aqua:` or `github:`).
- CI installs the same pins with `jdx/mise-action`, and every CI step is a
  mise task. Renovate bumps `rust-toolchain.toml` and `rust-version`
  together, and the `mise.toml` tools through its `mise` manager.

## Adding a command

Google adds endpoints rarely; most changes are new flags. For a new
endpoint, in this order (the tests in `tests/catalog.rs` fail until each
step is done):

1. Make sure it only reads. If it changes anything in Search Console, it
   does not belong here (decision 1).
2. In `src/api.rs`, add the `Endpoint` const, add it to `ENDPOINTS`, and add
   one `Api` method that builds its `ApiRequest`. Unit-test the URL and body.
3. In `src/cli/commands/<group>.rs`, add the clap args struct, a
   `pub(crate) static` `CommandSpec` (an `example` that dry-runs), and a
   `Leaf` impl. A one-request command calls `ctx.single(&SPEC, &request)`.
4. Add the leaf to its group enum's `leaf()`, and the group to `Command` in
   `src/cli/mod.rs` if it is new.
5. Add the registry entry in `src/cli/catalog.rs`.
6. Add mock tests in `tests/`: method, path, exact body, bearer token,
   stdout.
7. Run `UPDATE_DOCS=1 mise exec -- cargo test --test catalog docs_commands`
   and review the diff of `docs/commands.md`.

## Where tests go

- Request building, encoding, dates, paging, the join → a unit test in that
  module.
- A CLI flag, an exit code, an endpoint call → `tests/`, against the real
  binary and a mock Google (`--base-url`, and `token_uri` in a key file for
  the token endpoint).
- A new error path → a case in `tests/errors.rs` (or `tests/auth.rs` for
  credentials) that calls `assert_no_secret`.

## Conventions

- **Read-only by construction** (decision 1). `ApiRequest` is built only by
  `Api`'s methods. Do not add a public constructor, a method parameter, a
  raw request, or a scope other than `webmasters.readonly`.
- **stdout** (decision 3): Google's JSON unchanged for `sites`, `sitemaps`
  and `inspect`; rows with named keys for `query` and `compare`. camelCase
  keys, as in the API. Print through `output::print` or the macros, never
  `println!`.
- **Exit codes** (decision 5): **0** success (also when stdout closed
  early), **1** `api`, **2** `config` (clap usage errors keep clap's text),
  **3** `transport`/`timeout`/`decode`, **4** `auth`.
- **An error is exactly one JSON line on stderr and nothing on stdout:**
  `{"error":{"kind","message","hint","status","reason","body","request"}}`.
  Warnings are `{"warning":{"kind","message"}}`, and `--verbose` lines are
  JSON too.
- **Secrets never appear** (decision 6): not the access token, refresh
  token, client secret or private key, in any output, `Debug` included.
- **Flag names** (decision 8): the API field, hyphenated and lowercase.
  Enum values keep the exact API spelling, and each enum's `as_str` must
  equal its clap name (a unit test checks).
- **Dates** (decision 9): without `--end-date`, the probe finds the latest
  day with data. Keep it: a range that ends today compares empty days.
- Write in-code comments that describe why code does what it does, not
  what it does. The "what" should be self-evident. Be concise and direct.
  Cite the design decisions: "decision 9".
- Print through the macros in `src/cli/out.rs` (`out!`, `outln!`, `err!`,
  `errln!`), never `println!` or `eprintln!`.

## Gotchas

- **Never `env_clear()` in tests** (decision 14). It removes
  `LLVM_PROFILE_FILE`, and cargo-llvm-cov then loses the coverage of the
  spawned binary. `tests/common/mod.rs` removes the variables one by one
  and sets an empty `HOME`; add any new variable the CLI reads to its
  `ENV` list.
- **`serde_json::Map::remove` reorders keys** under `preserve_order` (it is
  `swap_remove`). Use `shift_remove`.
- **Google leaves `rows` out** when nothing matches, and `siteEntry` /
  `sitemap` out when the list is empty. Read them through `search::rows_of`
  or `Value::get`, never index.
- **The site is one path segment**, encoded like `encodeURIComponent`
  (`sc-domain%3Aexample.com`, `https%3A%2F%2Fexample.com%2F`). The API
  matches the property name exactly: `https://example.com` is not
  `https://example.com/`.
- **httptest:** a `Server` with no expectations fails on drop if any
  request arrives, which is how tests assert "nothing was sent".
  `expect_post` matches the JSON body exactly; `expect_probe` matches the
  probe without its dates, which depend on today. httptest's closure
  matchers need a sized input, so a decoded form needs the `Form` matcher
  in `tests/auth.rs`.
- **clippy pedantic** wants `Box<…>` for large subcommand variants
  (`Command::Query`), and splits a `run` over 100 lines (`too_many_lines`).
- **zsh:** a shell variable named `path` replaces `$PATH` (zsh ties the
  two). Do not use it in a loop.
