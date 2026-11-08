//! The flags clap answers before any command runs.

use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gsc-cli"))
        .args(args)
        .env_remove("GSC_SITE")
        .output()
        .unwrap()
}

#[test]
fn version_prints_the_binary_name_and_the_cargo_version() {
    let out = run(&["--version"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "gsc-cli 0.0.0\n");
}

#[test]
fn help_states_the_read_only_guarantee_and_the_exit_codes() {
    let out = run(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    let help = String::from_utf8(out.stdout).unwrap();
    assert!(help.contains("read-only"), "{help}");
    assert!(help.contains("webmasters.readonly"), "{help}");
    assert!(help.contains("4  no credentials"), "{help}");
}

#[test]
fn no_arguments_is_a_usage_error() {
    assert_eq!(run(&[]).status.code(), Some(2));
}

#[test]
fn global_numbers_outside_their_range_are_usage_errors() {
    for (flag, value) in [
        ("--timeout", "0"),
        ("--timeout", "3601"),
        ("--retries", "11"),
        ("--timeout", "x"),
    ] {
        let out = run(&["sites", "list", "--dry-run", flag, value]);
        assert_eq!(out.status.code(), Some(2), "{flag} {value}");
    }
}

#[test]
fn a_command_about_one_property_needs_the_site() {
    let out = run(&["sites", "get", "--dry-run"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--site <SITE>"));
}
