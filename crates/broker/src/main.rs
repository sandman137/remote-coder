//! SSH forced-command entrypoint. Reads $SSH_ORIGINAL_COMMAND, authorizes it
//! against the tmux-subcommand whitelist + session scope, then execs tmux.
//! Implemented in Phase 6; Phase 0 stub denies everything.

use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!("helm-broker: not yet implemented (Phase 6); denying by default");
    ExitCode::FAILURE
}
