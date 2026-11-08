//! The exit codes, as a type (decision 5).
//!
//! An enum rather than bare `u8` constants, because an agent branches on the
//! code: a credentials problem reported as a transport failure would send it
//! retrying instead of asking a person to log in.

use std::process::ExitCode;

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    /// 0: the result is on stdout. A stdout closed early (`| head`) is
    /// also a success, because the reader got what it wanted.
    Success,
    /// 1: Search Console answered with a non-2xx status. The request reached
    /// the API, so retrying the same request gives the same answer.
    Api,
    /// 2: the command line or the configuration is wrong: a clap usage error,
    /// a bad date, a bad base URL. No request was sent.
    Usage,
    /// 3: no usable answer came back: connect, TLS, a timeout, or a body that
    /// is not JSON. The same request may work on a retry.
    Transport,
    /// 4: no credentials, or Google refused them. A person must fix this
    /// (`gsc-cli auth login`, a new key, a missing scope); a retry will not.
    Auth,
}

impl ExitStatus {
    /// The process exit code.
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            ExitStatus::Success => 0,
            ExitStatus::Api => 1,
            ExitStatus::Usage => 2,
            ExitStatus::Transport => 3,
            ExitStatus::Auth => 4,
        }
    }
}

impl From<ExitStatus> for ExitCode {
    fn from(status: ExitStatus) -> ExitCode {
        ExitCode::from(status.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_codes_match_decision_5() {
        assert_eq!(ExitStatus::Success.code(), 0);
        assert_eq!(ExitStatus::Api.code(), 1);
        assert_eq!(ExitStatus::Usage.code(), 2);
        assert_eq!(ExitStatus::Transport.code(), 3);
        assert_eq!(ExitStatus::Auth.code(), 4);
        assert_eq!(ExitCode::from(ExitStatus::Auth), ExitCode::from(4));
    }
}
