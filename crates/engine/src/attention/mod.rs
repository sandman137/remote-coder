//! Tiered attention detection (DESIGN.md §9). This module is tier 3 — the
//! regex tier that works for *any* agent with zero host integration: adapter
//! patterns matched against pane text near the cursor. Tier 2
//! (monitor-silence) and tier 1 (agent hooks) arrive with the notifier
//! (Phase 7) and simply produce the same events with higher confidence.

use std::collections::HashMap;

use crate::adapter::AgentAdapter;
use crate::event::PromptKind;
use crate::grid::GridSnapshot;

/// How many rows above the cursor are considered "current" prompt territory
/// when the cursor is visible: inline prompts sit on the cursor row (or one
/// above, right after an echoed newline). Wider windows keep matching
/// already-answered prompts that are still on screen a few rows up.
const CURSOR_WINDOW_ROWS: u16 = 1;
/// An attention state must miss this many consecutive evaluations before it
/// clears — damps flapping while an answered prompt scrolls away.
const CLEAR_AFTER_MISSES: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionState {
    pub agent_id: String,
    pub kind: PromptKind,
    /// The text the pattern matched (for logging/tests; never pushed off-host).
    pub matched: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttentionUpdate {
    /// Pane transitioned to waiting (or the prompt changed).
    Waiting(AttentionState),
    /// Pane stopped waiting.
    Cleared,
}

#[derive(Debug, Default)]
struct PaneTrack {
    state: Option<AttentionState>,
    misses: u8,
}

/// Edge-triggered attention evaluation, one instance per streamer.
#[derive(Debug, Default)]
pub struct AttentionEngine {
    panes: HashMap<String, PaneTrack>,
}

impl AttentionEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Evaluate a pane's grid against its adapter. Returns an update only on
    /// state transitions.
    pub fn evaluate(
        &mut self,
        pane_id: &str,
        adapter: &AgentAdapter,
        grid: &GridSnapshot,
    ) -> Option<AttentionUpdate> {
        let window = attention_window(grid);
        let hit = adapter
            .attention
            .iter()
            .find_map(|re| re.find(&window).map(|m| m.as_str().to_string()));

        let track = self.panes.entry(pane_id.to_string()).or_default();
        match hit {
            Some(matched) => {
                track.misses = 0;
                let state = AttentionState {
                    agent_id: adapter.id.clone(),
                    kind: classify_prompt(&matched, &window),
                    matched,
                };
                if track.state.as_ref() == Some(&state) {
                    return None; // still waiting on the same prompt
                }
                track.state = Some(state.clone());
                Some(AttentionUpdate::Waiting(state))
            }
            None => {
                track.state.as_ref()?;
                track.misses += 1;
                if track.misses < CLEAR_AFTER_MISSES {
                    return None;
                }
                track.state = None;
                track.misses = 0;
                Some(AttentionUpdate::Cleared)
            }
        }
    }

    /// Extract metadata fields from the full visible text; returns only
    /// fields whose value changed since the last evaluation.
    pub fn extract_metadata(
        &mut self,
        pane_id: &str,
        adapter: &AgentAdapter,
        grid: &GridSnapshot,
        last: &mut HashMap<String, String>,
    ) -> HashMap<String, String> {
        let _ = pane_id;
        let text = grid.to_text();
        let mut changed = HashMap::new();
        for ex in &adapter.metadata {
            // Last match wins: the most recent occurrence is the current value.
            if let Some(caps) = ex.regex.captures_iter(&text).last() {
                if let Some(value) = caps.get(1).map(|m| m.as_str().to_string()) {
                    if last.get(&ex.field) != Some(&value) {
                        last.insert(ex.field.clone(), value.clone());
                        changed.insert(ex.field.clone(), value);
                    }
                }
            }
        }
        changed
    }

    pub fn current(&self, pane_id: &str) -> Option<&AttentionState> {
        self.panes.get(pane_id).and_then(|t| t.state.as_ref())
    }
}

/// The text region a *current* prompt could occupy. With a visible cursor
/// (line-oriented agents), only the rows at/just above it count — anything
/// higher is scrolled-past history that may contain already-answered
/// prompts. With a hidden cursor (full-screen TUIs that repaint completely,
/// like Claude Code menus), the whole visible grid is current by
/// construction.
fn attention_window(grid: &GridSnapshot) -> String {
    let (start, end) = match grid.cursor {
        Some((_, row)) => (row.saturating_sub(CURSOR_WINDOW_ROWS), row + 1),
        None => (0, grid.rows),
    };
    let mut out = String::new();
    for row in start..end.min(grid.rows) {
        out.push_str(&grid.row_text(row));
        out.push('\n');
    }
    out
}

/// Classify what kind of input the prompt wants, from the matched text and
/// its surroundings.
fn classify_prompt(matched: &str, window: &str) -> PromptKind {
    let lower = matched.to_lowercase();
    if lower.contains("y/n") || lower.contains("yes/no") {
        return PromptKind::YesNo;
    }
    let menu_re = regex::Regex::new(r"(?m)^\s*(?:❯\s*)?1[.)]\s+\S").unwrap();
    if menu_re.is_match(window) || menu_re.is_match(matched) {
        return PromptKind::Menu;
    }
    if lower.contains("waiting for") || lower.ends_with(':') {
        return PromptKind::FreeText;
    }
    PromptKind::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::Registry;
    use crate::grid::sgr;

    fn grid_of(text: &str, cursor: Option<(u16, u16)>) -> GridSnapshot {
        let mut g = sgr::parse_capture(text.as_bytes(), 80, 12);
        g.cursor = cursor;
        g
    }

    fn registry() -> Registry {
        Registry::load_builtins().unwrap()
    }

    #[test]
    fn yn_prompt_triggers_once_then_clears() {
        let reg = registry();
        let adapter = reg.get("fake-yn").unwrap();
        let mut eng = AttentionEngine::new();

        let waiting = grid_of("working…\nProceed? (y/n) ", Some((15, 1)));
        match eng.evaluate("%1", adapter, &waiting) {
            Some(AttentionUpdate::Waiting(s)) => {
                assert_eq!(s.agent_id, "fake-yn");
                assert_eq!(s.kind, PromptKind::YesNo);
                assert!(s.matched.contains("(y/n)"));
            }
            other => panic!("{other:?}"),
        }
        // Same prompt again: no re-fire.
        assert_eq!(eng.evaluate("%1", adapter, &waiting), None);

        // Prompt answered; needs CLEAR_AFTER_MISSES misses to clear.
        let moved_on = grid_of("proceeding…\nnext step", Some((9, 1)));
        assert_eq!(eng.evaluate("%1", adapter, &moved_on), None);
        assert_eq!(
            eng.evaluate("%1", adapter, &moved_on),
            Some(AttentionUpdate::Cleared)
        );
        assert_eq!(eng.evaluate("%1", adapter, &moved_on), None);
    }

    #[test]
    fn stale_prompt_above_cursor_window_is_ignored() {
        let reg = registry();
        let adapter = reg.get("fake-yn").unwrap();
        let mut eng = AttentionEngine::new();

        // Old prompt at row 0, cursor far below at row 8: not waiting.
        let text = "Proceed? (y/n)\nproceeding…\na\nb\nc\nd\ne\nf\ng";
        let g = grid_of(text, Some((0, 8)));
        assert_eq!(eng.evaluate("%1", adapter, &g), None);
    }

    #[test]
    fn hidden_cursor_uses_tail_window() {
        let reg = registry();
        let adapter = reg.get("fake-numbered").unwrap();
        let mut eng = AttentionEngine::new();

        let g = grid_of(
            "edit 3 ready: src/main.rs\n1) apply  2) skip  3) abort\n> ",
            None,
        );
        match eng.evaluate("%2", adapter, &g) {
            Some(AttentionUpdate::Waiting(s)) => {
                assert_eq!(s.agent_id, "fake-numbered");
                assert_eq!(s.kind, PromptKind::Menu);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn metadata_changes_are_edge_triggered() {
        let reg = registry();
        let adapter = reg.get("fake-yn").unwrap();
        let mut eng = AttentionEngine::new();
        let mut last = HashMap::new();

        let g1 = grid_of("✓ step complete (tokens: 137)\n", Some((0, 1)));
        let c1 = eng.extract_metadata("%1", adapter, &g1, &mut last);
        assert_eq!(c1.get("tokens").map(String::as_str), Some("137"));

        // Unchanged → no event fields.
        let c2 = eng.extract_metadata("%1", adapter, &g1, &mut last);
        assert!(c2.is_empty());

        // Newest occurrence wins.
        let g3 = grid_of(
            "✓ step complete (tokens: 137)\n✓ step complete (tokens: 274)\n",
            Some((0, 2)),
        );
        let c3 = eng.extract_metadata("%1", adapter, &g3, &mut last);
        assert_eq!(c3.get("tokens").map(String::as_str), Some("274"));
    }

    #[test]
    fn claude_style_menu_classifies_as_menu() {
        let reg = registry();
        let adapter = reg.get("claude-code").unwrap();
        let mut eng = AttentionEngine::new();
        let g = grid_of(
            "Do you want to proceed?\n❯ 1. Yes\n  2. Yes, and don't ask again\n  3. No\n",
            None,
        );
        match eng.evaluate("%3", adapter, &g) {
            Some(AttentionUpdate::Waiting(s)) => {
                assert_eq!(s.agent_id, "claude-code");
                assert_eq!(s.kind, PromptKind::Menu);
            }
            other => panic!("{other:?}"),
        }
    }
}
