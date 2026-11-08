//! `gsc-cli`: an unofficial, read-only command-line client for the Google
//! Search Console API, built for LLM agents. `docs/design-notes.md` holds the
//! numbered decisions that the comments here cite.

// A panicking library is a bug: this crate returns `Result` everywhere, and an
// agent reads the one JSON error line on stderr, not a backtrace. Declared here
// rather than in `Cargo.toml`, because a manifest `[lints]` table also covers
// `tests/`, where a panic *is* the failure report. `clippy.toml` exempts the
// unit tests in `src/`.
#![warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod cli;
pub mod output;
