//! GUI-owned preferences. Session/catalog state deliberately does not live here.
use std::collections::BTreeMap;

use griz_protocol::WorkspaceAccent;

use rune::{Chord, ChordKey, ChordMod, Key, KeyEvent, KeyPlatform, Tx, prefs::Prefs};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Font {
    #[default]
    Menlo,
    Monaco,
    System,
}

impl Font {
    pub const ALL: [Self; 3] = [Self::Menlo, Self::Monaco, Self::System];

    pub fn name(self) -> &'static str {
        match self {
            Self::Menlo => "Menlo",
            Self::Monaco => "Monaco",
            Self::System => "System Mono",
        }
    }

    pub fn family(self) -> rune::Family {
        match self {
            Self::Menlo => rune::Family::Named("Menlo"),
            Self::Monaco => rune::Family::Named("Monaco"),
            Self::System => rune::Family::SystemMono,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    Dark,
    Midnight,
}

impl Theme {
    pub const ALL: [Self; 2] = [Self::Dark, Self::Midnight];

    pub fn name(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Midnight => "Midnight",
        }
    }
}

/// GUI-only highlight compression; the service's terminal colors stay authoritative.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TerminalWhites {
    Original,
    #[default]
    Soft,
    Softer,
}

impl TerminalWhites {
    pub const ALL: [Self; 3] = [Self::Original, Self::Soft, Self::Softer];

    pub fn name(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::Soft => "Soft",
            Self::Softer => "Softer",
        }
    }

    pub fn white_point(self) -> u8 {
        match self {
            Self::Original => 255,
            Self::Soft => 216,
            Self::Softer => 200,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    NewTerminal,
    CloseTerminal,
    SplitHorizontal,
    SplitVertical,
    NextPane,
    PreviousPane,
    TogglePaneZoom,
    NewWorkspace,
    ToggleSidebar,
    NextTab,
    PreviousTab,
    IncreaseFontSize,
    DecreaseFontSize,
    QuickOpen,
    CommandPalette,
    NextWorkspace,
    PreviousWorkspace,
    SwitchWorkspace(u8),
    FocusPaneLeft,
    FocusPaneRight,
    FocusPaneUp,
    FocusPaneDown,
}

impl Action {
    pub const ALL: [Self; 30] = [
        Self::NewTerminal,
        Self::CloseTerminal,
        Self::NewWorkspace,
        Self::ToggleSidebar,
        Self::NextTab,
        Self::PreviousTab,
        Self::IncreaseFontSize,
        Self::DecreaseFontSize,
        Self::QuickOpen,
        Self::CommandPalette,
        Self::SplitHorizontal,
        Self::SplitVertical,
        Self::NextPane,
        Self::PreviousPane,
        Self::TogglePaneZoom,
        Self::NextWorkspace,
        Self::PreviousWorkspace,
        Self::SwitchWorkspace(1),
        Self::SwitchWorkspace(2),
        Self::SwitchWorkspace(3),
        Self::SwitchWorkspace(4),
        Self::SwitchWorkspace(5),
        Self::SwitchWorkspace(6),
        Self::SwitchWorkspace(7),
        Self::SwitchWorkspace(8),
        Self::SwitchWorkspace(9),
        Self::FocusPaneLeft,
        Self::FocusPaneRight,
        Self::FocusPaneUp,
        Self::FocusPaneDown,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::NewTerminal => "terminal.new",
            Self::CloseTerminal => "terminal.close",
            Self::SplitHorizontal => "pane.split_horizontal",
            Self::SplitVertical => "pane.split_vertical",
            Self::NextPane => "pane.next",
            Self::PreviousPane => "pane.previous",
            Self::TogglePaneZoom => "pane.toggle_zoom",
            Self::NewWorkspace => "workspace.new",
            Self::ToggleSidebar => "sidebar.toggle",
            Self::NextTab => "tab.next",
            Self::PreviousTab => "tab.previous",
            Self::IncreaseFontSize => "font.increase",
            Self::DecreaseFontSize => "font.decrease",
            Self::QuickOpen => "palette.navigate",
            Self::CommandPalette => "palette.commands",
            Self::NextWorkspace => "workspace.next",
            Self::PreviousWorkspace => "workspace.previous",
            Self::FocusPaneLeft => "pane.left",
            Self::FocusPaneRight => "pane.right",
            Self::FocusPaneUp => "pane.up",
            Self::FocusPaneDown => "pane.down",
            Self::SwitchWorkspace(n) => match n {
                1 => "workspace.1",
                2 => "workspace.2",
                3 => "workspace.3",
                4 => "workspace.4",
                5 => "workspace.5",
                6 => "workspace.6",
                7 => "workspace.7",
                8 => "workspace.8",
                9 => "workspace.9",
                _ => unreachable!("workspace shortcut index"),
            },
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::NewTerminal => "New terminal",
            Self::CloseTerminal => "Close active terminal",
            Self::SplitHorizontal => "Split horizontally (top / bottom)",
            Self::SplitVertical => "Split vertically (left / right)",
            Self::NextPane => "Focus next pane",
            Self::PreviousPane => "Focus previous pane",
            Self::TogglePaneZoom => "Toggle pane zoom",
            Self::NewWorkspace => "New workspace…",
            Self::ToggleSidebar => "Toggle sidebar",
            Self::NextTab => "Next tab",
            Self::PreviousTab => "Previous tab",
            Self::IncreaseFontSize => "Increase app font size",
            Self::DecreaseFontSize => "Decrease app font size",
            Self::QuickOpen => "Jump to workspace or terminal…",
            Self::CommandPalette => "Command palette…",
            Self::NextWorkspace => "Next workspace",
            Self::PreviousWorkspace => "Previous workspace",
            Self::FocusPaneLeft => "Focus pane left",
            Self::FocusPaneRight => "Focus pane right",
            Self::FocusPaneUp => "Focus pane up",
            Self::FocusPaneDown => "Focus pane down",
            Self::SwitchWorkspace(n) => match n {
                1 => "Switch to workspace 1",
                2 => "Switch to workspace 2",
                3 => "Switch to workspace 3",
                4 => "Switch to workspace 4",
                5 => "Switch to workspace 5",
                6 => "Switch to workspace 6",
                7 => "Switch to workspace 7",
                8 => "Switch to workspace 8",
                9 => "Switch to workspace 9",
                _ => unreachable!("workspace shortcut index"),
            },
        }
    }

    pub fn default_chord(self) -> Option<Chord> {
        Some(
            Chord::parse(match self {
                Self::NewTerminal => "mod+t",
                Self::CloseTerminal => "mod+w",
                Self::SplitHorizontal => "mod+shift+d",
                Self::SplitVertical => "mod+d",
                Self::NextPane => "mod+alt+right",
                Self::PreviousPane => "mod+alt+left",
                Self::TogglePaneZoom => "mod+enter",
                Self::NewWorkspace => "mod+shift+n",
                Self::ToggleSidebar => "mod+b",
                Self::NextTab => "mod+shift+right",
                Self::PreviousTab => "mod+shift+left",
                Self::IncreaseFontSize => "mod+plus",
                Self::DecreaseFontSize => "mod+minus",
                Self::QuickOpen => "mod+p",
                Self::CommandPalette => "mod+shift+p",
                _ => return None,
            })
            .expect("built-in shortcut"),
        )
    }

    pub fn default_prefix_chord(self) -> Option<Chord> {
        Some(
            Chord::parse(match self {
                Self::PreviousWorkspace => "p",
                Self::NextWorkspace => "n",
                Self::SwitchWorkspace(n @ 1..=9) => {
                    return Some(Chord::new(ChordKey::Char(char::from(b'0' + n))));
                }
                Self::PreviousTab => "<",
                Self::NextTab => ">",
                Self::SplitHorizontal => "\"",
                Self::SplitVertical => "%",
                Self::FocusPaneLeft => "h",
                Self::FocusPaneDown => "j",
                Self::FocusPaneUp => "k",
                Self::FocusPaneRight => "l",
                Self::TogglePaneZoom => "z",
                _ => return None,
            })
            .expect("built-in prefix shortcut"),
        )
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub font: Font,
    pub font_size: f32,
    pub theme: Theme,
    pub terminal_whites: TerminalWhites,
    pub default_workspace_accent: WorkspaceAccent,
    /// Missing means the default; an empty string explicitly disables a shortcut.
    bindings: BTreeMap<String, String>,
    prefix: Option<Chord>,
    prefix_bindings: BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font: Font::default(),
            font_size: 14.0,
            theme: Theme::default(),
            terminal_whites: TerminalWhites::default(),
            default_workspace_accent: WorkspaceAccent::Green,
            bindings: BTreeMap::new(),
            prefix: Some(Chord::parse("ctrl+b").unwrap()),
            prefix_bindings: BTreeMap::new(),
        }
    }
}

impl Settings {
    /// A shared type scale: preserve the hierarchy between labels, headings and terminal text.
    pub fn ui_font(&self, mut style: rune::TextStyle) -> rune::TextStyle {
        let scale = self.font_size / 14.0;
        style.size *= scale;
        style.line_height *= scale;
        style.tracking *= scale;
        style
    }

    pub fn matches(&self, action: Action, event: &KeyEvent, tx: &Tx) -> bool {
        self.chord(action).is_some_and(|chord| {
            chord.matches_tx(event, tx)
                // The unshifted '=' key is the conventional alias for '+' on US keyboards.
                || (action == Action::IncreaseFontSize
                    && action.default_chord().is_some_and(|c| chord.same_keys(&c, KeyPlatform::current()))
                    && Chord::parse("mod+equal").unwrap().matches_tx(event, tx))
        })
    }

    pub fn from_store(prefs: &Prefs) -> (Self, Option<String>) {
        let payload = prefs
            .get::<serde_json::Value>("settings")
            .unwrap_or_else(|| serde_json::json!({}));
        let Some(values) = payload.as_object() else {
            return (
                Self::default(),
                Some("Saved settings could not be read; using defaults.".into()),
            );
        };
        let mut settings = Self::default();
        let mut invalid = false;
        if let Some(value) = values.get("font") {
            if let Some(font) = value
                .as_str()
                .and_then(|name| Font::ALL.into_iter().find(|font| font.name() == name))
            {
                settings.font = font;
            } else {
                invalid = true;
            }
        }
        if let Some(value) = values.get("font_size") {
            if let Some(size) = value
                .as_f64()
                .map(|size| size as f32)
                .filter(|size| size.is_finite() && (10.0..=28.0).contains(size))
            {
                settings.font_size = size;
            } else {
                invalid = true;
            }
        }
        if let Some(value) = values.get("theme") {
            if let Some(theme) = value
                .as_str()
                .and_then(|name| Theme::ALL.into_iter().find(|theme| theme.name() == name))
            {
                settings.theme = theme;
            } else {
                invalid = true;
            }
        }
        if let Some(value) = values.get("terminal_whites") {
            if let Some(whites) = value.as_str().and_then(|name| {
                TerminalWhites::ALL
                    .into_iter()
                    .find(|whites| whites.name() == name)
            }) {
                settings.terminal_whites = whites;
            } else {
                invalid = true;
            }
        }
        if let Some(value) = values.get("default_workspace_accent") {
            match serde_json::from_value::<WorkspaceAccent>(value.clone()) {
                Ok(accent) => settings.default_workspace_accent = accent,
                Err(_) => invalid = true,
            }
        }
        // Load direct overrides before validating the prefix, so disabled/swapped
        // defaults and pre-prefix versions' Ctrl+B bindings remain intact.
        settings.prefix = None;
        if let Some(value) = values.get("prefix_bindings") {
            if let Ok(bindings) = serde_json::from_value::<BTreeMap<String, String>>(value.clone())
            {
                for action in Action::ALL {
                    if bindings.contains_key(action.id()) {
                        settings
                            .prefix_bindings
                            .insert(action.id().into(), String::new());
                    }
                }
                for action in Action::ALL {
                    if let Some(spec) = bindings.get(action.id()) {
                        let chord = if spec.is_empty() {
                            Ok(None)
                        } else {
                            Chord::parse(spec).map(Some)
                        };
                        match chord {
                            Ok(chord) if settings.bind_prefix(action, chord).is_ok() => {}
                            _ => invalid = true,
                        }
                    }
                }
            } else {
                invalid = true;
            }
        }
        if let Some(value) = values.get("keybindings") {
            if let Ok(bindings) = serde_json::from_value::<BTreeMap<String, String>>(value.clone())
            {
                // Validate against defaults and earlier overrides in a deterministic order.
                // Clear known overrides first so swaps between two actions can load correctly.
                for action in Action::ALL {
                    if bindings.contains_key(action.id()) {
                        settings.bindings.insert(action.id().into(), String::new());
                    }
                }
                for action in Action::ALL {
                    if let Some(spec) = bindings.get(action.id()) {
                        let chord = if spec.is_empty() {
                            Ok(None)
                        } else {
                            Chord::parse(spec).map(Some)
                        };
                        match chord {
                            Ok(chord) if settings.bind(action, chord).is_ok() => {}
                            _ => invalid = true,
                        }
                    }
                }
            } else {
                invalid = true;
            }
        }
        let prefix = match values.get("prefix_key") {
            Some(value) => match value.as_str().map(|s| {
                if s.is_empty() {
                    Ok(None)
                } else {
                    Chord::parse(s).map(Some)
                }
            }) {
                Some(Ok(chord)) => chord,
                _ => {
                    invalid = true;
                    Self::default().prefix
                }
            },
            None => Self::default().prefix,
        };
        if settings.set_prefix(prefix).is_err() {
            // Prefer keeping an existing direct shortcut to silently replacing it.
            invalid = true;
        }
        (
            settings,
            invalid.then(|| "Some saved settings were invalid and were ignored. Review Settings before making changes.".into()),
        )
    }

    pub fn save(&self, prefs: &mut Prefs) -> Result<(), String> {
        // One atomic write, while preserving keys from future versions of Griz.
        let mut value = prefs
            .get::<BTreeMap<String, serde_json::Value>>("settings")
            .unwrap_or_default();
        value.insert("font".into(), self.font.name().into());
        value.insert("font_size".into(), self.font_size.into());
        value.insert("theme".into(), self.theme.name().into());
        value.insert("terminal_whites".into(), self.terminal_whites.name().into());
        // The style picker has been retired; all workspace buttons are Soft.
        value.remove("workspace_button_style");
        value.insert(
            "default_workspace_accent".into(),
            serde_json::to_value(self.default_workspace_accent).unwrap(),
        );
        value.insert(
            "keybindings".into(),
            serde_json::to_value(&self.bindings).unwrap(),
        );
        value.insert(
            "prefix_key".into(),
            self.prefix
                .map(|c| c.to_string())
                .unwrap_or_default()
                .into(),
        );
        value.insert(
            "prefix_bindings".into(),
            serde_json::to_value(&self.prefix_bindings).unwrap(),
        );
        prefs.set("settings", &value).map_err(|e| e.to_string())
    }

    pub fn chord(&self, action: Action) -> Option<Chord> {
        match self.bindings.get(action.id()) {
            Some(spec) if spec.is_empty() => None,
            Some(spec) => Chord::parse(spec).ok(),
            None => action.default_chord(),
        }
    }

    pub fn label(&self, action: Action) -> String {
        self.chord(action)
            .map(|chord| chord.label())
            .unwrap_or_else(|| "Not set".into())
    }

    pub fn bind(&mut self, action: Action, chord: Option<Chord>) -> Result<(), String> {
        if let Some(chord) = chord {
            validate_chord(chord)?;
            if self
                .prefix
                .is_some_and(|p| bindings_overlap(action, chord, action, p))
            {
                return Err("That shortcut is already used by the prefix key.".into());
            }
            if let Some(other) = Action::ALL.into_iter().find(|other| {
                *other != action
                    && self
                        .chord(*other)
                        .is_some_and(|c| bindings_overlap(action, chord, *other, c))
            }) {
                return Err(format!(
                    "{} is already used by {}.",
                    chord.label(),
                    other.label()
                ));
            }
        }
        self.bindings.insert(
            action.id().into(),
            chord.map(|c| c.to_string()).unwrap_or_default(),
        );
        Ok(())
    }

    pub fn prefix(&self) -> Option<Chord> {
        self.prefix
    }

    pub fn set_prefix(&mut self, chord: Option<Chord>) -> Result<(), String> {
        if let Some(chord) = chord {
            validate_chord(chord)?;
            if let Some(action) = Action::ALL.into_iter().find(|action| {
                self.chord(*action)
                    .is_some_and(|c| bindings_overlap(*action, c, *action, chord))
            }) {
                return Err(format!(
                    "{} is already used by {}.",
                    chord.label(),
                    action.label()
                ));
            }
        }
        self.prefix = chord;
        Ok(())
    }

    pub fn prefix_chord(&self, action: Action) -> Option<Chord> {
        match self.prefix_bindings.get(action.id()) {
            Some(spec) if spec.is_empty() => None,
            Some(spec) => Chord::parse(spec).ok(),
            None => action.default_prefix_chord(),
        }
    }

    pub fn prefix_label(&self, action: Action) -> String {
        self.prefix_chord(action)
            .map(|c| format!("Prefix → {}", c.label()))
            .unwrap_or_else(|| "Not set".into())
    }

    pub fn bind_prefix(&mut self, action: Action, chord: Option<Chord>) -> Result<(), String> {
        if let Some(chord) = chord {
            if chord.key() == Chord::parse("escape").unwrap().key() {
                return Err("Escape is reserved for canceling the prefix.".into());
            }
            validate_reserved(chord)?;
            if let Some(other) = Action::ALL.into_iter().find(|other| {
                *other != action
                    && self
                        .prefix_chord(*other)
                        .is_some_and(|c| c.same_keys(&chord, KeyPlatform::current()))
            }) {
                return Err(format!(
                    "Prefix → {} is already used by {}.",
                    chord.label(),
                    other.label()
                ));
            }
        }
        self.prefix_bindings.insert(
            action.id().into(),
            chord.map(|c| c.to_string()).unwrap_or_default(),
        );
        Ok(())
    }

    pub fn reset_prefix_binding(&mut self, action: Action) -> Result<(), String> {
        self.bind_prefix(action, action.default_prefix_chord())?;
        self.prefix_bindings.remove(action.id());
        Ok(())
    }

    pub fn reset_binding(&mut self, action: Action) -> Result<(), String> {
        self.bind(action, action.default_chord())?;
        self.bindings.remove(action.id());
        Ok(())
    }
}

fn bindings_overlap(action: Action, chord: Chord, other: Action, other_chord: Chord) -> bool {
    let alternatives = |action: Action, chord: Chord| {
        let mut chords = vec![chord];
        if action == Action::IncreaseFontSize
            && action
                .default_chord()
                .is_some_and(|c| chord.same_keys(&c, KeyPlatform::current()))
        {
            chords.push(Chord::parse("mod+equal").unwrap());
            chords.push(Chord::parse("mod+shift+equal").unwrap());
        }
        chords
    };
    let others = alternatives(other, other_chord);
    alternatives(action, chord).iter().any(|a| {
        others
            .iter()
            .any(|b| a.same_keys(b, KeyPlatform::current()))
    })
}

fn validate_chord(chord: Chord) -> Result<(), String> {
    if ![
        ChordMod::Primary,
        ChordMod::Ctrl,
        ChordMod::Alt,
        ChordMod::Super,
    ]
    .into_iter()
    .any(|m| chord.has(m))
    {
        return Err("Include Command, Control, or Option so normal typing stays available.".into());
    }
    validate_reserved(chord)
}

fn validate_reserved(chord: Chord) -> Result<(), String> {
    // Keep Settings reachable and preserve native window/editing commands. These may be
    // handled by AppKit before Rune sees a key, so offering them would be misleading.
    if [
        "mod+comma",
        "mod+q",
        "mod+m",
        "mod+h",
        "mod+alt+h",
        "mod+c",
        "mod+v",
        "mod+x",
        "mod+a",
        "mod+z",
        "mod+shift+z",
    ]
    .into_iter()
    .any(|spec| chord.same_keys(&Chord::parse(spec).unwrap(), KeyPlatform::current()))
    {
        return Err(format!(
            "{} is reserved for Settings or a system/editing command.",
            chord.label()
        ));
    }
    Ok(())
}

/// Capture layout-aware logical chords, not just US physical key positions.
pub fn record(event: &KeyEvent, tx: &Tx) -> Result<Chord, String> {
    let chord = capture_chord(event, tx, false)?;
    validate_chord(chord)?;
    Ok(chord)
}

/// Prefix suffixes use the typed punctuation (e.g. '"' rather than Shift+apostrophe).
pub fn record_suffix(event: &KeyEvent, tx: &Tx) -> Result<Chord, String> {
    capture_chord(event, tx, true)
}

fn capture_chord(event: &KeyEvent, tx: &Tx, literal: bool) -> Result<Chord, String> {
    let mut typed_symbol = false;
    let key = match &event.key {
        Key::Text(text) => {
            let mut chars = text.chars();
            let c = chars
                .next()
                .filter(|_| chars.next().is_none())
                .ok_or("Press a single key with modifiers.")?;
            typed_symbol = literal && !c.is_alphanumeric();
            ChordKey::Char(if typed_symbol {
                c
            } else {
                tx.key_base().unwrap_or(c)
            })
        }
        Key::Command(c) => ChordKey::Char(tx.key_base().unwrap_or(*c)),
        Key::Function(n) => ChordKey::Function(*n),
        other => Chord::parse(match other {
            Key::Enter => "enter",
            Key::Tab => "tab",
            Key::Backspace => "backspace",
            Key::Delete => "delete",
            Key::Insert => "insert",
            Key::Home => "home",
            Key::End => "end",
            Key::PageUp => "pageup",
            Key::PageDown => "pagedown",
            Key::Left => "left",
            Key::Right => "right",
            Key::Up => "up",
            Key::Down => "down",
            _ => return Err("That key cannot be used as a shortcut.".into()),
        })
        .unwrap()
        .key(),
    };
    let m = event.mods;
    let mut chord = Chord::new(key);
    let primary = m.cmd
        || if cfg!(target_os = "macos") {
            m.logo
        } else {
            m.ctrl
        };
    if primary {
        chord = chord.with(ChordMod::Primary);
    }
    if m.ctrl && cfg!(target_os = "macos") {
        chord = chord.with(ChordMod::Ctrl);
    }
    if m.logo && !cfg!(target_os = "macos") {
        chord = chord.with(ChordMod::Super);
    }
    if m.alt {
        chord = chord.with(ChordMod::Alt);
    }
    if m.shift && !typed_symbol {
        chord = chord.with(ChordMod::Shift);
    }
    Ok(chord)
}
