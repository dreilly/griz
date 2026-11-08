//! GUI-owned preferences. Session/catalog state deliberately does not live here.
use std::collections::BTreeMap;

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkspaceButtonStyle {
    #[default]
    Soft,
    Outline,
    Solid,
    Ghost,
    Pill,
    Accent,
}

impl WorkspaceButtonStyle {
    pub const ALL: [Self; 6] = [
        Self::Soft,
        Self::Outline,
        Self::Solid,
        Self::Ghost,
        Self::Pill,
        Self::Accent,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Soft => "soft",
            Self::Outline => "outline",
            Self::Solid => "solid",
            Self::Ghost => "ghost",
            Self::Pill => "pill",
            Self::Accent => "accent",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Soft => "Soft",
            Self::Outline => "Outline",
            Self::Solid => "Solid",
            Self::Ghost => "Ghost",
            Self::Pill => "Pill",
            Self::Accent => "Accent",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    NewTerminal,
    CloseTerminal,
    NewWorkspace,
    ToggleSidebar,
    NextTab,
    PreviousTab,
    IncreaseFontSize,
    DecreaseFontSize,
}

impl Action {
    pub const ALL: [Self; 8] = [
        Self::NewTerminal,
        Self::CloseTerminal,
        Self::NewWorkspace,
        Self::ToggleSidebar,
        Self::NextTab,
        Self::PreviousTab,
        Self::IncreaseFontSize,
        Self::DecreaseFontSize,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::NewTerminal => "terminal.new",
            Self::CloseTerminal => "terminal.close",
            Self::NewWorkspace => "workspace.new",
            Self::ToggleSidebar => "sidebar.toggle",
            Self::NextTab => "tab.next",
            Self::PreviousTab => "tab.previous",
            Self::IncreaseFontSize => "font.increase",
            Self::DecreaseFontSize => "font.decrease",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::NewTerminal => "New terminal",
            Self::CloseTerminal => "Close active terminal…",
            Self::NewWorkspace => "New workspace…",
            Self::ToggleSidebar => "Toggle sidebar",
            Self::NextTab => "Next tab",
            Self::PreviousTab => "Previous tab",
            Self::IncreaseFontSize => "Increase app font size",
            Self::DecreaseFontSize => "Decrease app font size",
        }
    }

    pub fn default_chord(self) -> Chord {
        Chord::parse(match self {
            Self::NewTerminal => "mod+t",
            Self::CloseTerminal => "mod+w",
            Self::NewWorkspace => "mod+shift+n",
            Self::ToggleSidebar => "mod+b",
            Self::NextTab => "mod+shift+right",
            Self::PreviousTab => "mod+shift+left",
            Self::IncreaseFontSize => "mod+plus",
            Self::DecreaseFontSize => "mod+minus",
        })
        .expect("built-in shortcut")
    }
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub font: Font,
    pub font_size: f32,
    pub theme: Theme,
    pub workspace_button_style: WorkspaceButtonStyle,
    /// Missing means the default; an empty string explicitly disables a shortcut.
    bindings: BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font: Font::default(),
            font_size: 14.0,
            theme: Theme::default(),
            workspace_button_style: WorkspaceButtonStyle::default(),
            bindings: BTreeMap::new(),
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
                    && chord.same_keys(&action.default_chord(), KeyPlatform::current())
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
        if let Some(value) = values.get("workspace_button_style") {
            if let Some(style) = value.as_str().and_then(|id| {
                WorkspaceButtonStyle::ALL
                    .into_iter()
                    .find(|style| style.id() == id)
            }) {
                settings.workspace_button_style = style;
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
        value.insert(
            "workspace_button_style".into(),
            self.workspace_button_style.id().into(),
        );
        value.insert(
            "keybindings".into(),
            serde_json::to_value(&self.bindings).unwrap(),
        );
        prefs.set("settings", &value).map_err(|e| e.to_string())
    }

    pub fn chord(&self, action: Action) -> Option<Chord> {
        match self.bindings.get(action.id()) {
            Some(spec) if spec.is_empty() => None,
            Some(spec) => Chord::parse(spec).ok(),
            None => Some(action.default_chord()),
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

    pub fn reset_binding(&mut self, action: Action) -> Result<(), String> {
        self.bind(action, Some(action.default_chord()))?;
        self.bindings.remove(action.id());
        Ok(())
    }
}

fn bindings_overlap(action: Action, chord: Chord, other: Action, other_chord: Chord) -> bool {
    let alternatives = |action: Action, chord: Chord| {
        let mut chords = vec![chord];
        if action == Action::IncreaseFontSize
            && chord.same_keys(&action.default_chord(), KeyPlatform::current())
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
    let key = match &event.key {
        Key::Text(text) => {
            let mut chars = text.chars();
            let c = chars
                .next()
                .filter(|_| chars.next().is_none())
                .ok_or("Press a single key with modifiers.")?;
            ChordKey::Char(tx.key_base().unwrap_or(c))
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
    if m.shift {
        chord = chord.with(ChordMod::Shift);
    }
    validate_chord(chord)?;
    Ok(chord)
}
