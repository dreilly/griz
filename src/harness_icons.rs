//! Shared, bundled harness marks. Identity comes only from OSC 7501's `app`,
//! never titles, terminal output, process names, or program-supplied SVG/URLs.
use crate::ui::Palette;
use griz_protocol::{TabInfo, WorkspaceInfo};
use rune::{El, Icon, icon};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Harness {
    Pi,
    Claude,
    Codex,
    Gemini,
    OpenCode,
    Other,
}

static PI: Icon = Icon {
    name: "harness-pi",
    view: 470.0,
    body: include_str!("../assets/harnesses/pi.svg"),
};
static CLAUDE: Icon = Icon {
    name: "harness-claude",
    view: 24.0,
    body: include_str!("../assets/harnesses/claude.svg"),
};
static CODEX: Icon = Icon {
    name: "harness-codex",
    view: 24.0,
    body: include_str!("../assets/harnesses/openai.svg"),
};
static GEMINI: Icon = Icon {
    name: "harness-gemini",
    view: 24.0,
    body: include_str!("../assets/harnesses/gemini.svg"),
};
static OPENCODE: Icon = Icon {
    name: "harness-opencode",
    view: 24.0,
    body: include_str!("../assets/harnesses/opencode.svg"),
};
static PROGRAM: Icon = Icon {
    name: "harness-program",
    view: 24.0,
    body: r#"<rect x="3" y="4" width="18" height="16" rx="3" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M7 9l3 3-3 3m6 0h4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

impl Harness {
    pub(crate) fn from_app(app: &str) -> Self {
        match app.trim().to_ascii_lowercase().as_str() {
            "pi" | "pi-coding-agent" => Self::Pi,
            "claude" | "claude-code" | "claude_code" | "claude code" => Self::Claude,
            "codex" | "codex-cli" | "codex_cli" | "codex cli" => Self::Codex,
            "gemini" | "gemini-cli" | "gemini_cli" | "gemini cli" => Self::Gemini,
            "opencode" | "open-code" | "open_code" => Self::OpenCode,
            _ => Self::Other,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Pi => "Pi",
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Gemini => "Gemini CLI",
            Self::OpenCode => "OpenCode",
            Self::Other => "Program",
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::OpenCode => "opencode",
            Self::Other => "program",
        }
    }

    pub(crate) fn badge(self, key: impl Into<String>, p: &Palette) -> El {
        let mark = match self {
            Self::Pi => &PI,
            Self::Claude => &CLAUDE,
            Self::Codex => &CODEX,
            Self::Gemini => &GEMINI,
            Self::OpenCode => &OPENCODE,
            Self::Other => &PROGRAM,
        };
        icon(mark, p.label.line_px().max(16.0))
            .key(key)
            .label(self.name())
            .help(self.name())
            .color(p.ink)
    }
}

/// A split tab represents every pane, not just the selected one. Child records
/// and multiple panes using the same harness must not repeat its mark.
pub(crate) fn for_tab(workspace: &WorkspaceInfo, tab: &TabInfo) -> Vec<Harness> {
    let mut harnesses = Vec::new();
    for id in tab.layout.terminal_ids() {
        if let Some(terminal) = workspace.terminals.iter().find(|t| t.id == id) {
            for agent in &terminal.agents {
                let harness = Harness::from_app(&agent.record.app);
                if !harnesses.contains(&harness) {
                    harnesses.push(harness);
                }
            }
        }
    }
    harnesses
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_svg_marks_have_visible_coverage_at_ui_sizes() {
        // Element existence alone cannot catch unsupported SVG metadata hiding
        // geometry in Rune's monochrome rasterizer.
        for mark in [&PI, &CLAUDE, &CODEX, &GEMINI, &OPENCODE, &PROGRAM] {
            for size in [16, 32] {
                let coverage = rune::text::icon_coverage(mark, size);
                let visible = coverage.iter().filter(|&&alpha| alpha > 0).count();
                assert!(
                    visible > (size * size / 8) as usize,
                    "{} at {size}px is blank or too small",
                    mark.name
                );
            }
        }
    }

    #[test]
    fn matches_explicit_app_aliases_only() {
        for (app, expected) in [
            ("pi", Harness::Pi),
            (" Pi-Coding-Agent ", Harness::Pi),
            ("CLAUDE", Harness::Claude),
            ("claude-code", Harness::Claude),
            ("Claude Code", Harness::Claude),
            ("codex-cli", Harness::Codex),
            ("gemini_cli", Harness::Gemini),
            ("OpenCode", Harness::OpenCode),
            ("", Harness::Other),
            ("terraform", Harness::Other),
            ("not-pi", Harness::Other),
            ("pi<script>", Harness::Other),
            ("https://pi.dev/logo-auto.svg", Harness::Other),
            ("pi\u{202e}", Harness::Other),
        ] {
            assert_eq!(Harness::from_app(app), expected, "{app:?}");
        }
    }
}
