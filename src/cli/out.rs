//! Every write the CLI makes, and the four macros the commands write through.
//!
//! The macros are `#[macro_export]`ed because `src/main.rs` is a separate crate
//! from the library and also prints through them. So they live at the crate
//! root: `use crate::outln;` inside the library, `gsc_cli::errln` outside.

/// Writes to stdout, with a closed pipe treated as the end of the run.
///
/// `println!` panics when the write fails, and a reader that quits early
/// (`gsc-cli query --all --format jsonl | head`) closes the pipe
/// under the writer. Rust ignores `SIGPIPE`, so that write comes back as
/// `ErrorKind::BrokenPipe` rather than killing the process, and restoring the
/// default handler needs `libc` and an `unsafe` block the crate forbids. So the
/// output goes through here instead: a closed pipe ends the output quietly and
/// leaves the exit code alone.
///
/// Returns `false` when stdout is closed, so that paging stops fetching pages
/// nobody will read (decision 10).
#[expect(
    clippy::must_use_candidate,
    reason = "a command that prints one value has nothing to do when stdout is closed; only the `--all` loop reads the result"
)]
pub fn write_out(args: std::fmt::Arguments) -> bool {
    write_to(&mut std::io::stdout().lock(), args, "stdout")
}

/// Writes to stderr. A closed stderr leaves nobody to tell, so the result is
/// dropped.
pub fn write_err(args: std::fmt::Arguments) {
    write_to(&mut std::io::stderr().lock(), args, "stderr");
}

fn write_to(w: &mut impl std::io::Write, args: std::fmt::Arguments, name: &str) -> bool {
    match w.write_fmt(args) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => false,
        // A full disk, say. Say so on the other stream, and carry on.
        Err(e) => {
            let _ = std::io::Write::write_fmt(
                &mut std::io::stderr(),
                format_args!("gsc-cli: cannot write to {name}: {e}\n"),
            );
            true
        }
    }
}

/// `print!`, `println!`, `eprint!` and `eprintln!`, routed through `write_out`
/// and `write_err`. The CLI uses these four and never the standard ones.
///
/// The bodies name `$crate::cli::out` rather than the imported functions, so a
/// caller only has to import the macro it uses.
#[macro_export]
macro_rules! out {
    ($($arg:tt)*) => { $crate::cli::out::write_out(format_args!($($arg)*)) };
}

/// `println!` through [`write_out`]. Evaluates to `false` when stdout is
/// closed.
#[macro_export]
macro_rules! outln {
    () => { $crate::cli::out::write_out(format_args!("\n")) };
    ($($arg:tt)*) => {
        $crate::cli::out::write_out(format_args!("{}\n", format_args!($($arg)*)))
    };
}

/// `eprint!` through [`write_err`].
#[macro_export]
macro_rules! err {
    ($($arg:tt)*) => { $crate::cli::out::write_err(format_args!($($arg)*)) };
}

/// `eprintln!` through [`write_err`].
#[macro_export]
macro_rules! errln {
    ($($arg:tt)*) => {
        $crate::cli::out::write_err(format_args!("{}\n", format_args!($($arg)*)))
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, ErrorKind, Write};

    struct Failing(ErrorKind);

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(self.0))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_successful_write_reports_open() {
        let mut buf = Vec::new();
        assert!(write_to(&mut buf, format_args!("a{}", 1), "test"));
        assert_eq!(buf, b"a1");
    }

    #[test]
    fn a_closed_pipe_reports_closed() {
        let mut w = Failing(ErrorKind::BrokenPipe);
        assert!(!write_to(&mut w, format_args!("x"), "test"));
        assert!(w.flush().is_ok());
    }

    #[test]
    fn any_other_failure_is_reported_and_the_run_carries_on() {
        let mut w = Failing(ErrorKind::StorageFull);
        assert!(write_to(&mut w, format_args!("x"), "test"));
    }
}
