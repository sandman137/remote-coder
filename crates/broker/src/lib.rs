//! Forced-command broker (DESIGN.md §8.2) — the least-privilege trust
//! boundary. A paired device's key can *only* run this binary; the broker
//! tokenizes `$SSH_ORIGINAL_COMMAND`, checks it against a tmux-subcommand
//! whitelist, scopes every `-t` target to the configured session, and execs
//! tmux. Everything else is denied. All scoping is enforced host-side; the
//! client is untrusted (§13).
//!
//! Honest caveat (§8.2): `send-keys` types into whatever pane it targets —
//! if an agent pane sits at a shell prompt, keys reach the shell. The
//! broker's containment is *session scoping* and *no shell channels*; it
//! does not sandbox agent behavior itself.
//!
//! The allow/deny core is pure (no SSH, no tmux) and unit-tested (§10.4);
//! `main.rs` supplies the tmux-backed pane resolver and the final exec.

pub mod enroll;

/// Resolves `%pane` targets to their owning session (the layout of `%N`
/// carries no session info, so scoping needs a lookup).
pub trait PaneResolver {
    fn session_of_pane(&self, pane_id: &str) -> Option<String>;
}

impl<F: Fn(&str) -> Option<String>> PaneResolver for F {
    fn session_of_pane(&self, pane_id: &str) -> Option<String> {
        self(pane_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Exec tmux with exactly this argv (the leading `tmux` word stripped).
    Allowed(Vec<String>),
    Denied(String),
}

impl Decision {
    pub fn allowed(&self) -> bool {
        matches!(self, Decision::Allowed(_))
    }
}

/// POSIX-ish shell tokenizer covering what our transport emits (and what a
/// human types): single quotes (literal), double quotes (with backslash
/// escapes for `"` and `\`), bare-word backslash escapes. No expansion of
/// any kind — `$`, backticks, globs stay literal, which is safe because the
/// result is exec'd as argv, never re-parsed by a shell.
pub fn tokenize(cmd: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = cmd.chars().peekable();
    let mut in_word = false;

    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => cur.push(c),
                        None => return Err("unterminated single quote".into()),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(e @ ('"' | '\\' | '$' | '`')) => cur.push(e),
                            Some(e) => {
                                cur.push('\\');
                                cur.push(e);
                            }
                            None => return Err("unterminated double quote".into()),
                        },
                        Some(c) => cur.push(c),
                        None => return Err("unterminated double quote".into()),
                    }
                }
            }
            '\\' => {
                in_word = true;
                match chars.next() {
                    Some(e) => cur.push(e),
                    None => return Err("trailing backslash".into()),
                }
            }
            _ => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        out.push(cur);
    }
    Ok(out)
}

/// Is `target` (a `-t` argument) inside the session scope?
fn target_in_scope(target: &str, session: &str, resolver: &dyn PaneResolver) -> bool {
    let t = target.strip_prefix('=').unwrap_or(target);
    if let Some(pane) = t.strip_prefix('%') {
        return resolver
            .session_of_pane(&format!("%{pane}"))
            .is_some_and(|s| s == session);
    }
    if t.starts_with('@') {
        return false; // window-ids carry no session info; engine never sends them
    }
    let session_part = t.split(':').next().unwrap_or(t);
    session_part == session
}

/// tmux format strings run `#(shell command)` — a format argument from the
/// network must never carry one.
fn format_is_safe(fmt: &str) -> bool {
    !fmt.contains("#(")
}

struct ArgScan<'a> {
    toks: &'a [String],
    i: usize,
}

impl<'a> ArgScan<'a> {
    fn new(toks: &'a [String]) -> Self {
        ArgScan { toks, i: 0 }
    }
    fn next(&mut self) -> Option<&'a str> {
        let t = self.toks.get(self.i).map(String::as_str);
        if t.is_some() {
            self.i += 1;
        }
        t
    }
    fn value(&mut self, flag: &str) -> Result<&'a str, String> {
        self.next().ok_or_else(|| format!("{flag} needs a value"))
    }
    fn rest(&mut self) -> &'a [String] {
        let r = &self.toks[self.i..];
        self.i = self.toks.len();
        r
    }
}

/// The §8.2 whitelist. `original` is `$SSH_ORIGINAL_COMMAND` as received.
pub fn authorize(original: &str, session: &str, resolver: &dyn PaneResolver) -> Decision {
    let toks = match tokenize(original) {
        Ok(t) => t,
        Err(e) => return Decision::Denied(format!("unparseable command: {e}")),
    };
    if toks.is_empty() {
        return Decision::Denied("empty command (interactive shell request?)".into());
    }
    if toks[0] != "tmux" {
        return Decision::Denied(format!("only tmux is brokered, got {:?}", toks[0]));
    }

    let deny = |why: String| Decision::Denied(why);
    let mut scan = ArgScan::new(&toks[1..]);
    let mut argv: Vec<String> = Vec::new();

    // Optional control-mode flag before the subcommand.
    let mut sub = match scan.next() {
        Some(s) => s,
        None => return deny("missing tmux subcommand".into()),
    };
    if sub == "-C" || sub == "-CC" {
        argv.push(sub.to_string());
        sub = match scan.next() {
            Some(s) => s,
            None => return deny("missing subcommand after -C".into()),
        };
        if !matches!(sub, "attach-session" | "attach") {
            return deny(format!("-C only valid with attach-session, got {sub:?}"));
        }
    }
    argv.push(sub.to_string());

    // Per-subcommand flag grammar. Every -t is scope-checked; every
    // format-expanded value is checked for #() execution.
    let scoped = |scan: &mut ArgScan, argv: &mut Vec<String>, flag: &str| -> Result<(), String> {
        let v = scan.value(flag)?;
        if !target_in_scope(v, session, resolver) {
            return Err(format!("target {v:?} outside session {session:?}"));
        }
        argv.push(flag.into());
        argv.push(v.into());
        Ok(())
    };
    let fmt_arg = |scan: &mut ArgScan, argv: &mut Vec<String>, flag: &str| -> Result<(), String> {
        let v = scan.value(flag)?;
        if !format_is_safe(v) {
            return Err(format!("{flag} value contains #() command substitution"));
        }
        argv.push(flag.into());
        argv.push(v.into());
        Ok(())
    };

    let result: Result<(), String> = (|| {
        match sub {
            "list-sessions" => {
                while let Some(t) = scan.next() {
                    match t {
                        "-F" => fmt_arg(&mut scan, &mut argv, "-F")?,
                        other => return Err(format!("list-sessions: flag {other:?} not allowed")),
                    }
                }
            }
            "list-windows" | "list-panes" => {
                let mut saw_target = false;
                while let Some(t) = scan.next() {
                    match t {
                        "-F" => fmt_arg(&mut scan, &mut argv, "-F")?,
                        "-s" => argv.push("-s".into()),
                        "-t" => {
                            scoped(&mut scan, &mut argv, "-t")?;
                            saw_target = true;
                        }
                        "-a" => {
                            return Err(format!("{sub}: -a (all sessions) not allowed"));
                        }
                        other => return Err(format!("{sub}: flag {other:?} not allowed")),
                    }
                }
                if !saw_target {
                    return Err(format!("{sub}: -t <session target> is required"));
                }
            }
            "capture-pane" => {
                let mut saw_target = false;
                while let Some(t) = scan.next() {
                    match t {
                        "-p" => argv.push("-p".into()),
                        "-e" => argv.push("-e".into()),
                        "-t" => {
                            scoped(&mut scan, &mut argv, "-t")?;
                            saw_target = true;
                        }
                        "-S" | "-E" => {
                            let v = scan.value(t)?;
                            if !v.chars().all(|c| c.is_ascii_digit() || c == '-') {
                                return Err(format!("capture-pane: bad {t} value {v:?}"));
                            }
                            argv.push(t.into());
                            argv.push(v.into());
                        }
                        other => return Err(format!("capture-pane: flag {other:?} not allowed")),
                    }
                }
                if !saw_target {
                    return Err("capture-pane: -t is required".into());
                }
            }
            "send-keys" => {
                let mut saw_target = false;
                while let Some(t) = scan.next() {
                    match t {
                        "-t" => {
                            scoped(&mut scan, &mut argv, "-t")?;
                            saw_target = true;
                        }
                        "-l" => argv.push("-l".into()),
                        "--" => {
                            if !saw_target {
                                return Err("send-keys: -t must precede keys".into());
                            }
                            argv.push("--".into());
                            for k in scan.rest() {
                                argv.push(k.clone());
                            }
                        }
                        other if !other.starts_with('-') => {
                            if !saw_target {
                                return Err("send-keys: -t must precede keys".into());
                            }
                            argv.push(other.into());
                        }
                        other => return Err(format!("send-keys: flag {other:?} not allowed")),
                    }
                }
                if !saw_target {
                    return Err("send-keys: -t is required".into());
                }
            }
            "display-message" => {
                let mut saw_target = false;
                while let Some(t) = scan.next() {
                    match t {
                        "-p" => argv.push("-p".into()),
                        "-t" => {
                            scoped(&mut scan, &mut argv, "-t")?;
                            saw_target = true;
                        }
                        msg if !msg.starts_with('-') => {
                            if !format_is_safe(msg) {
                                return Err(
                                    "display-message: message contains #() substitution".into()
                                );
                            }
                            argv.push(msg.into());
                        }
                        other => {
                            return Err(format!("display-message: flag {other:?} not allowed"))
                        }
                    }
                }
                if !saw_target {
                    return Err("display-message: -t is required".into());
                }
            }
            "attach-session" | "attach" => {
                let mut saw_target = false;
                while let Some(t) = scan.next() {
                    match t {
                        "-t" => {
                            scoped(&mut scan, &mut argv, "-t")?;
                            saw_target = true;
                        }
                        other => return Err(format!("attach-session: flag {other:?} not allowed")),
                    }
                }
                if !saw_target {
                    return Err("attach-session: -t is required".into());
                }
            }
            "resize-window" => {
                let mut saw_target = false;
                while let Some(t) = scan.next() {
                    match t {
                        "-t" => {
                            scoped(&mut scan, &mut argv, "-t")?;
                            saw_target = true;
                        }
                        "-x" | "-y" => {
                            let v = scan.value(t)?;
                            if !v.chars().all(|c| c.is_ascii_digit()) {
                                return Err(format!("resize-window: bad {t} value {v:?}"));
                            }
                            argv.push(t.into());
                            argv.push(v.into());
                        }
                        other => return Err(format!("resize-window: flag {other:?} not allowed")),
                    }
                }
                if !saw_target {
                    return Err("resize-window: -t is required".into());
                }
            }
            "refresh-client" => {
                // Client size only ("-C WxH"); no targets, no other flags.
                while let Some(t) = scan.next() {
                    match t {
                        "-C" => {
                            let v = scan.value("-C")?;
                            let ok = v.split_once('x').is_some_and(|(w, h)| {
                                !w.is_empty()
                                    && !h.is_empty()
                                    && w.chars().all(|c| c.is_ascii_digit())
                                    && h.chars().all(|c| c.is_ascii_digit())
                            });
                            if !ok {
                                return Err(format!("refresh-client: bad -C size {v:?}"));
                            }
                            argv.push("-C".into());
                            argv.push(v.into());
                        }
                        other => return Err(format!("refresh-client: flag {other:?} not allowed")),
                    }
                }
            }
            "new-window" => {
                // Launching agents in-scope. Same §8.2 caveat as send-keys:
                // this is command execution by design, contained to the
                // scoped session + revocable keys.
                let mut saw_target = false;
                while let Some(t) = scan.next() {
                    match t {
                        "-t" => {
                            scoped(&mut scan, &mut argv, "-t")?;
                            saw_target = true;
                        }
                        "-n" => {
                            let v = scan.value("-n")?;
                            argv.push("-n".into());
                            argv.push(v.into());
                        }
                        "-P" => argv.push("-P".into()),
                        "-F" => fmt_arg(&mut scan, &mut argv, "-F")?,
                        "-c" => {
                            let v = scan.value("-c")?;
                            argv.push("-c".into());
                            argv.push(v.into());
                        }
                        cmd if !cmd.starts_with('-') => {
                            if !saw_target {
                                return Err("new-window: -t must precede the command".into());
                            }
                            argv.push(cmd.into());
                        }
                        other => return Err(format!("new-window: flag {other:?} not allowed")),
                    }
                }
                if !saw_target {
                    return Err("new-window: -t is required".into());
                }
            }
            other => return Err(format!("tmux subcommand {other:?} not allowed")),
        }
        Ok(())
    })();

    match result {
        Ok(()) => Decision::Allowed(argv),
        Err(why) => deny(why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn resolver() -> impl PaneResolver {
        let map: HashMap<String, String> = [
            ("%5".to_string(), "agents".to_string()),
            ("%7".to_string(), "other".to_string()),
        ]
        .into();
        move |pane: &str| map.get(pane).cloned()
    }

    fn auth(cmd: &str) -> Decision {
        authorize(cmd, "agents", &resolver())
    }

    // §10.4 — the design's own cases.
    #[test]
    fn broker_denies_shell() {
        assert!(!auth("bash -i").allowed());
    }

    #[test]
    fn broker_scopes_session() {
        assert!(auth("tmux capture-pane -t agents:0.0 -p").allowed());
        assert!(!auth("tmux capture-pane -t other:0.0 -p").allowed());
    }

    #[test]
    fn denies_empty_and_non_tmux() {
        assert!(!auth("").allowed());
        assert!(!auth("id").allowed());
        assert!(!auth("tmux").allowed());
        assert!(!auth("rm -rf /").allowed());
    }

    #[test]
    fn denies_unlisted_subcommands() {
        for cmd in [
            "tmux kill-server",
            "tmux kill-session -t agents",
            "tmux run-shell 'id'",
            "tmux set-option -g default-shell /bin/sh",
            "tmux pipe-pane -t agents:0.0 'cat > /tmp/x'",
            "tmux source-file /tmp/evil.conf",
        ] {
            assert!(!auth(cmd).allowed(), "must deny: {cmd}");
        }
    }

    #[test]
    fn pane_id_targets_resolve_through_resolver() {
        assert!(auth("tmux send-keys -t %5 -l -- y").allowed());
        assert!(!auth("tmux send-keys -t %7 -l -- y").allowed());
        assert!(!auth("tmux send-keys -t %99 -l -- y").allowed()); // unknown
    }

    #[test]
    fn equals_prefix_and_subtargets_stay_scoped() {
        assert!(auth("tmux attach-session -t =agents").allowed());
        assert!(auth("tmux -C attach-session -t =agents").allowed());
        assert!(auth("tmux capture-pane -p -e -t agents:yn.0").allowed());
        assert!(!auth("tmux attach-session -t =agentsX").allowed());
        assert!(!auth("tmux attach-session -t other").allowed());
        // A sneaky prefix: "agents-evil" is NOT the agents session.
        assert!(!auth("tmux capture-pane -p -t agents-evil:0.0").allowed());
    }

    #[test]
    fn list_commands_require_scope_and_reject_all_flag() {
        assert!(auth("tmux list-panes -s -t =agents -F fmt").allowed());
        assert!(!auth("tmux list-panes -a -F fmt").allowed());
        assert!(!auth("tmux list-panes -F fmt").allowed()); // no target
        assert!(auth("tmux list-sessions -F '#{session_name}'").allowed());
    }

    #[test]
    fn format_command_substitution_is_rejected() {
        assert!(!auth("tmux list-sessions -F '#(rm -rf ~)'").allowed());
        assert!(!auth("tmux display-message -p -t agents '#(id)'").allowed());
        assert!(!auth("tmux new-window -t =agents -P -F '#(id)' sleep").allowed());
        // Plain #{} expansions are fine.
        assert!(auth("tmux display-message -p -t agents '#{pane_width}'").allowed());
    }

    #[test]
    fn capture_pane_flags_are_read_only() {
        assert!(auth("tmux capture-pane -p -e -t agents:0.0 -S -500").allowed());
        assert!(!auth("tmux capture-pane -p -t agents:0.0 -b buf").allowed());
        assert!(!auth("tmux capture-pane -p -t agents:0.0 -S '$(id)'").allowed());
    }

    #[test]
    fn send_keys_literals_pass_after_scoped_target() {
        assert!(auth("tmux send-keys -t agents:0.0 -l -- 'rm -rf /'").allowed());
        assert!(auth("tmux send-keys -t %5 -- Enter C-c").allowed());
        assert!(!auth("tmux send-keys -l -- y").allowed()); // no target
                                                            // Flag smuggling after -- is inert (it's argv, not shell), but a
                                                            // flag BEFORE the target is refused.
        assert!(!auth("tmux send-keys -X cancel -t agents:0.0").allowed());
    }

    #[test]
    fn refresh_and_resize_grammar() {
        assert!(auth("tmux refresh-client -C 100x30").allowed());
        assert!(!auth("tmux refresh-client -C evil").allowed());
        assert!(!auth("tmux refresh-client -t other").allowed());
        assert!(auth("tmux resize-window -t =agents -x 80 -y 24").allowed());
        assert!(!auth("tmux resize-window -t other -x 80 -y 24").allowed());
        assert!(!auth("tmux resize-window -t =agents -x '$(id)'").allowed());
    }

    #[test]
    fn allowed_argv_is_faithful() {
        match auth("tmux capture-pane -p -e -t 'agents:0.0' -S -100") {
            Decision::Allowed(argv) => {
                assert_eq!(
                    argv,
                    vec!["capture-pane", "-p", "-e", "-t", "agents:0.0", "-S", "-100"]
                );
            }
            d => panic!("{d:?}"),
        }
    }

    #[test]
    fn tokenizer_handles_transport_quoting() {
        // What SshTransport actually emits.
        let toks =
            tokenize("tmux 'list-panes' -s -t '=agents' -F '#{session_name}\u{1f}#{pane_id}'")
                .unwrap();
        assert_eq!(toks[0], "tmux");
        assert_eq!(toks[4], "=agents");
        assert!(toks[6].contains('\u{1f}'));
        assert_eq!(tokenize(r#"a "b c" d"#).unwrap(), vec!["a", "b c", "d"]);
        // shell_quote("it's") emits 'it'\''s' — must round-trip.
        assert_eq!(tokenize(r#"'it'\''s'"#).unwrap(), vec!["it's"]);
        assert!(tokenize("unterminated 'quote").is_err());
    }
}
