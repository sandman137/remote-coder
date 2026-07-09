//! Agent adapters (DESIGN.md §6): declarative TOML profiles adding the
//! *semantic sugar* per agent — launch command, attention patterns, quick
//! action buttons, metadata extractors. The mechanical layer (transport /
//! grid / keys) never needs per-agent code; adding an agent is a config
//! drop, not an engine change.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Deserialize;

use crate::error::EngineError;
use crate::event::Button;

/// Built-in profiles, embedded at compile time (user files override by id).
/// fake-yn / fake-numbered are the dev fixtures — kept as built-ins so the
/// TUI dev loop has agent buttons out of the box; they never match real
/// commands.
const BUILTINS: &[&str] = &[
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../adapters/claude-code.toml"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../adapters/codex.toml"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../adapters/cursor.toml"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../adapters/fake-yn.toml"
    )),
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../adapters/fake-numbered.toml"
    )),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AdapterTransport {
    #[default]
    Tmux,
    /// Agent Client Protocol — reserved (§6.4), not implemented.
    Acp,
}

#[derive(Debug, Clone)]
pub struct LaunchSpec {
    pub cmd: String,
    pub args: Vec<String>,
    /// "picker" = ask the user for a directory; otherwise a fixed path.
    pub cwd: CwdPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CwdPolicy {
    Picker,
    Fixed(String),
}

#[derive(Debug, Clone)]
pub struct MetaExtractor {
    pub field: String,
    pub regex: Regex,
}

#[derive(Debug, Clone)]
pub struct AgentAdapter {
    pub id: String,
    pub name: String,
    pub launch: LaunchSpec,
    /// Tier-3 attention patterns, matched against recent pane text.
    pub attention: Vec<Regex>,
    pub buttons: Vec<Button>,
    pub metadata: Vec<MetaExtractor>,
    /// Tier-1 host-side hook script name (Phase 7), if any.
    pub hook: Option<String>,
    pub transport: AdapterTransport,
}

// ---- raw TOML shape ----

#[derive(Deserialize)]
struct RawAdapter {
    id: String,
    name: String,
    launch: RawLaunch,
    #[serde(default)]
    attention: Vec<String>,
    #[serde(default)]
    buttons: Vec<RawButton>,
    #[serde(default)]
    metadata: Vec<RawMeta>,
    #[serde(default)]
    hook: String,
    #[serde(default)]
    transport: Option<String>,
}

#[derive(Deserialize)]
struct RawLaunch {
    cmd: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Deserialize)]
struct RawButton {
    label: String,
    keys: String,
}

#[derive(Deserialize)]
struct RawMeta {
    field: String,
    regex: String,
}

fn parse_adapter(toml_text: &str, origin: &str) -> Result<AgentAdapter, EngineError> {
    let raw: RawAdapter = toml::from_str(toml_text)
        .map_err(|e| EngineError::Parse(format!("adapter {origin}: {e}")))?;
    let compile = |pat: &str| {
        Regex::new(pat)
            .map_err(|e| EngineError::Parse(format!("adapter {origin} regex {pat:?}: {e}")))
    };
    let attention = raw
        .attention
        .iter()
        .map(|p| compile(p))
        .collect::<Result<Vec<_>, _>>()?;
    let metadata = raw
        .metadata
        .iter()
        .map(|m| {
            Ok(MetaExtractor {
                field: m.field.clone(),
                regex: compile(&m.regex)?,
            })
        })
        .collect::<Result<Vec<_>, EngineError>>()?;
    let transport = match raw.transport.as_deref() {
        None | Some("tmux") => AdapterTransport::Tmux,
        Some("acp") => AdapterTransport::Acp,
        Some(other) => {
            return Err(EngineError::Parse(format!(
                "adapter {origin}: unknown transport {other:?}"
            )))
        }
    };
    Ok(AgentAdapter {
        id: raw.id,
        name: raw.name,
        launch: LaunchSpec {
            cmd: raw.launch.cmd,
            args: raw.launch.args,
            cwd: match raw.launch.cwd.as_deref() {
                None | Some("picker") => CwdPolicy::Picker,
                Some(path) => CwdPolicy::Fixed(path.to_string()),
            },
        },
        attention,
        buttons: raw
            .buttons
            .into_iter()
            .map(|b| Button {
                label: b.label,
                keys: b.keys,
            })
            .collect(),
        metadata,
        hook: (!raw.hook.is_empty()).then_some(raw.hook),
        transport,
    })
}

/// Adapter registry: built-ins + user overrides, id-keyed, iteration in
/// stable (sorted) order so detection is deterministic.
#[derive(Debug, Default)]
pub struct Registry {
    adapters: BTreeMap<String, AgentAdapter>,
}

impl Registry {
    pub fn load_builtins() -> Result<Self, EngineError> {
        let mut adapters = BTreeMap::new();
        for (i, text) in BUILTINS.iter().enumerate() {
            let adapter = parse_adapter(text, &format!("builtin[{i}]"))?;
            adapters.insert(adapter.id.clone(), adapter);
        }
        Ok(Registry { adapters })
    }

    /// Built-ins overlaid with `$XDG_CONFIG_HOME/helm/adapters/*.toml`
    /// (user files win on id collision). Unreadable/broken user files are
    /// skipped with a warning — a typo must not brick the engine.
    pub fn load_builtins_and_overrides() -> Result<Self, EngineError> {
        let dir = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|base| base.join("helm/adapters"));
        Self::load_with_overrides(dir.as_deref())
    }

    /// Same, with an explicit override dir (testable).
    pub fn load_with_overrides(dir: Option<&Path>) -> Result<Self, EngineError> {
        let mut registry = Self::load_builtins()?;
        let Some(dir) = dir else {
            return Ok(registry);
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Ok(registry); // no override dir = no overrides
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect();
        paths.sort();
        for path in paths {
            match std::fs::read_to_string(&path)
                .map_err(|e| EngineError::Parse(format!("{}: {e}", path.display())))
                .and_then(|text| parse_adapter(&text, &path.display().to_string()))
            {
                Ok(adapter) => {
                    registry.adapters.insert(adapter.id.clone(), adapter);
                }
                Err(e) => tracing::warn!(error = %e, "skipping bad adapter file"),
            }
        }
        Ok(registry)
    }

    pub fn get(&self, id: &str) -> Option<&AgentAdapter> {
        self.adapters.get(id)
    }

    pub fn all(&self) -> impl Iterator<Item = &AgentAdapter> {
        self.adapters.values()
    }

    /// Best-effort auto-detect (§6.2): first by the pane's foreground
    /// command (basename match against launch cmd), then by attention
    /// patterns against recent pane text — the adapter with the *longest*
    /// match wins, so a specific prompt ("Proceed? (y/n)") beats another
    /// agent's generic pattern ("(y/n)"). Ties break alphabetically.
    pub fn detect(&self, current_command: &str, recent_text: &str) -> Option<&AgentAdapter> {
        let base = current_command
            .rsplit('/')
            .next()
            .unwrap_or(current_command);
        if let Some(hit) = self.adapters.values().find(|a| {
            let launch_base = a.launch.cmd.rsplit('/').next().unwrap_or(&a.launch.cmd);
            launch_base == base
        }) {
            return Some(hit);
        }
        if recent_text.is_empty() {
            return None;
        }
        self.adapters
            .values()
            .filter_map(|a| {
                let longest = a
                    .attention
                    .iter()
                    .filter_map(|re| re.find(recent_text).map(|m| m.len()))
                    .max()?;
                Some((a, longest))
            })
            .max_by(|(a, la), (b, lb)| la.cmp(lb).then(b.id.cmp(&a.id)))
            .map(|(a, _)| a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_load_and_contain_shipped_agents() {
        let r = Registry::load_builtins().unwrap();
        for id in ["claude-code", "codex", "cursor", "fake-yn", "fake-numbered"] {
            assert!(r.get(id).is_some(), "missing builtin {id}");
        }
        let cursor = r.get("cursor").unwrap();
        assert_eq!(cursor.name, "Cursor CLI");
        assert_eq!(cursor.launch.cmd, "agent");
        assert_eq!(cursor.launch.cwd, CwdPolicy::Picker);
        assert!(cursor.buttons.iter().any(|b| b.label == "Yes"));
        assert!(cursor.metadata.iter().any(|m| m.field == "cost"));
        assert_eq!(cursor.transport, AdapterTransport::Tmux);
        // claude-code ships a tier-1 hook name.
        assert!(r.get("claude-code").unwrap().hook.is_some());
    }

    #[test]
    fn override_wins_by_id_and_bad_files_are_skipped() {
        let dir = std::env::temp_dir().join(format!("helm-adapters-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("cursor.toml"),
            r#"
id = "cursor"
name = "Cursor (custom)"
launch = { cmd = "my-agent" }
attention = ['custom-prompt\?']
[[buttons]]
label = "Go"
keys = "g"
"#,
        )
        .unwrap();
        std::fs::write(dir.join("broken.toml"), "not toml at all [[").unwrap();

        let r = Registry::load_with_overrides(Some(&dir)).unwrap();
        let cursor = r.get("cursor").unwrap();
        assert_eq!(cursor.name, "Cursor (custom)");
        assert_eq!(cursor.launch.cmd, "my-agent");
        assert_eq!(cursor.buttons.len(), 1);
        // Built-ins unaffected by the broken file.
        assert!(r.get("claude-code").is_some());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detect_by_command_then_by_text() {
        let r = Registry::load_builtins().unwrap();
        assert_eq!(r.detect("claude", "").unwrap().id, "claude-code");
        assert_eq!(r.detect("/usr/bin/codex", "").unwrap().id, "codex");
        assert_eq!(r.detect("agent", "").unwrap().id, "cursor");
        assert_eq!(r.detect("fake-yn.sh", "").unwrap().id, "fake-yn");
        assert!(r.detect("bash", "").is_none());
        // Text fallback: the most specific (longest) match wins, so
        // fake-yn's full prompt beats other agents' generic '(y/n)'.
        assert_eq!(
            r.detect("bash", "…\nProceed? (y/n) ").unwrap().id,
            "fake-yn"
        );
        assert!(r.detect("bash", "just normal output").is_none());
    }

    #[test]
    fn rejects_unknown_transport() {
        let err = parse_adapter(
            r#"
id = "x"
name = "X"
launch = { cmd = "x" }
transport = "carrier-pigeon"
"#,
            "test",
        );
        assert!(err.is_err());
    }
}
