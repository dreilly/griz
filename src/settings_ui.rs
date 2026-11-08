//! Settings presentation; the typed model and persistence stay in settings.rs.
use crate::{
    State,
    settings::{self, Action, Font, Settings, Theme, WorkspaceButtonStyle},
    ui::{Palette, button, workspace_button},
};
use rune::{Color, El, Key, KeyEvent, TextStyle, Tx, col, el, row, text, text_input};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Appearance,
    Buttons,
    Shortcuts,
}

pub fn record_key(state: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(action) = state.recording else {
        return false;
    };
    if tx.key_repeat() {
        return true;
    }
    if event.key == Key::Escape {
        state.recording = None;
        state.settings_error = None;
    } else {
        match settings::record(event, tx) {
            Ok(chord) => state.bind_shortcut(action, Some(chord)),
            Err(error) => state.settings_error = Some(error),
        }
    }
    true
}

fn heading(title: &str, hint: &str, p: &Palette) -> El {
    col()
        .w_full()
        .gap(6.0)
        .child(text(title).font(p.title).color(p.ink))
        .child(text(hint).font(p.label).color(p.muted).wrap())
}

fn appearance(state: &State, p: &Palette) -> El {
    let mut fonts = row().w_full().flex_wrap().gap(6.0);
    for (index, font) in Font::ALL.into_iter().enumerate() {
        fonts = fonts.child(
            button(format!("settings-font-{index}"), font.name(), p)
                .fill(if state.settings.font == font {
                    p.accent.alpha(0.15)
                } else {
                    p.button_fill()
                })
                .on_press(move |s: &mut State| {
                    s.settings.font = font;
                    s.settings_changed();
                }),
        );
    }
    let mut themes = row().w_full().flex_wrap().gap(6.0);
    for (index, theme) in Theme::ALL.into_iter().enumerate() {
        themes = themes.child(
            button(format!("settings-theme-{index}"), theme.name(), p)
                .fill(if state.settings.theme == theme {
                    p.accent.alpha(0.15)
                } else {
                    p.button_fill()
                })
                .on_press(move |s: &mut State| {
                    s.settings.theme = theme;
                    s.settings_changed();
                }),
        );
    }
    col()
        .w_full()
        .gap(20.0)
        .child(heading(
            "Appearance",
            "Font size applies throughout the app. Changes apply immediately and save automatically.",
            p,
        ))
        .child(
            col()
                .gap(8.0)
                .child(text("Terminal font").font(p.label).color(p.ink))
                .child(fonts),
        )
        .child(
            col()
                .gap(8.0)
                .child(text("App font size").font(p.label).color(p.ink))
                .child(
                    row()
                        .gap(10.0)
                        .center_y()
                        .child(
                            button("settings-size-down", "−", p)
                                .disabled(state.settings.font_size <= 10.0)
                                .on_press(|s: &mut State| {
                                    s.change_font_size(-1.0);
                                }),
                        )
                        .child(
                            text(format!("{} pt", state.settings.font_size))
                                .font(p.label)
                                .color(p.ink),
                        )
                        .child(
                            button("settings-size-up", "+", p)
                                .disabled(state.settings.font_size >= 28.0)
                                .on_press(|s: &mut State| {
                                    s.change_font_size(1.0);
                                }),
                        ),
                ),
        )
        .child(
            col()
                .gap(8.0)
                .child(text("Theme").font(p.label).color(p.ink))
                .child(themes),
        )
        .child(
            col()
                .key("settings-preview")
                .w_full()
                .padding(14.0)
                .gap(6.0)
                .radius(6.0)
                .fill(crate::terminal_view::background_for(state.settings.theme))
                .child(
                    text("griz ~ % echo hello")
                        .font(
                            TextStyle::new(state.settings.font_size, 400.0)
                                .family(state.settings.font.family()),
                        )
                        .color(p.ink),
                )
                .child(
                    text("hello, world")
                        .font(
                            TextStyle::new(state.settings.font_size, 400.0)
                                .family(state.settings.font.family()),
                        )
                        .color(p.accent),
                ),
        )
        .child(
            button(
                "settings-appearance-reset",
                "Restore appearance defaults",
                p,
            )
            .on_press(|s: &mut State| {
                let defaults = Settings::default();
                s.settings.font = defaults.font;
                s.settings.font_size = defaults.font_size;
                s.settings.theme = defaults.theme;
                s.settings.workspace_button_style = defaults.workspace_button_style;
                s.settings_changed();
            }),
        )
}

fn buttons(state: &State, p: &Palette) -> El {
    let mut samples = row().w_full().flex_wrap().gap(12.0);
    for style in WorkspaceButtonStyle::ALL {
        let selected = state.settings.workspace_button_style == style;
        samples = samples.child(
            col()
                .key(format!("button-style-card-{}", style.id()))
                .w(230.0)
                .grow(1.0)
                .min_w(230.0)
                .padding(12.0)
                .gap(12.0)
                .radius(8.0)
                .border(1.0, if selected { p.ink.alpha(0.45) } else { p.line })
                .child(
                    row()
                        .w_full()
                        .center_y()
                        .child(text(style.name()).font(p.label).color(p.ink))
                        .child(el().grow(1.0))
                        .child(
                            text(if selected { "Selected" } else { "" })
                                .font(p.label)
                                .color(p.muted),
                        ),
                )
                .child(
                    workspace_button(
                        format!("button-style-{}", style.id()),
                        style,
                        p,
                        format!("button-style-label-{}", style.id()),
                    )
                    .w_full()
                    .label(format!("Use {} workspace button style", style.name()))
                    .on_press(move |s: &mut State| {
                        s.settings.workspace_button_style = style;
                        s.settings_changed();
                    }),
                ),
        );
    }
    col().w_full().gap(18.0)
        .child(heading(
            "Workspace button styles",
            "Click a sample to select and save its style. Samples never create a workspace. All but Accent are neutral.",
            p,
        ))
        .child(samples)
        .child(button("settings-buttons-reset", "Restore default button style", p)
            .on_press(|s: &mut State| {
                s.settings.workspace_button_style = WorkspaceButtonStyle::default();
                s.settings_changed();
            }))
}

fn shortcuts(state: &State, p: &Palette) -> El {
    let mut list = col().w_full().gap(8.0);
    let search = state.shortcut_search.text.to_lowercase();
    let mut count = 0;
    for action in Action::ALL {
        if !action.label().to_lowercase().contains(&search) && !action.id().contains(&search) {
            continue;
        }
        count += 1;
        let recording = state.recording == Some(action);
        list = list.child(
            col()
                .w_full()
                .gap(4.0)
                .child(text(action.label()).font(p.label).color(p.ink))
                .child(
                    row()
                        .w_full()
                        .gap(6.0)
                        .child(
                            button(
                                format!("shortcut-record-{}", action.id()),
                                &if recording {
                                    "Press shortcut…".into()
                                } else {
                                    state.settings.label(action)
                                },
                                p,
                            )
                            .grow(1.0)
                            .fill(if recording {
                                p.accent.alpha(0.18)
                            } else {
                                p.button_fill()
                            })
                            .on_press_tx(move |s: &mut State, tx| {
                                s.recording = Some(action);
                                s.settings_error = None;
                                tx.blur();
                            }),
                        )
                        .child(
                            button(format!("shortcut-clear-{}", action.id()), "Clear", p)
                                .on_press(move |s: &mut State| s.bind_shortcut(action, None)),
                        )
                        .child(
                            button(format!("shortcut-reset-{}", action.id()), "Reset", p).on_press(
                                move |s: &mut State| match s.settings.reset_binding(action) {
                                    Ok(()) => {
                                        s.recording = None;
                                        s.settings_changed();
                                    }
                                    Err(error) => s.settings_error = Some(error),
                                },
                            ),
                        ),
                ),
        );
    }
    if count == 0 {
        list = list.child(text("No matching actions").font(p.label).color(p.muted));
    }
    col().w_full().gap(14.0)
        .child(heading("Keyboard shortcuts", "Click a shortcut and press a new combination. Escape cancels recording. Font size and Close Active Terminal are app-wide; other actions apply with terminal focus.", p))
        .child(text_input(&state.shortcut_search, p.input)
            .key("settings-shortcut-search").label("Search shortcuts").single_line()
            .w_full().padding(8.0).radius(5.0).fill(p.ink.alpha(0.06)).color(p.ink)
            .placeholder("Search actions…", p.muted)
            .disabled(state.recording.is_some())
            .on_edit(|s: &mut State| &mut s.shortcut_search))
        .child(list)
        .child(text(format!("{} always opens Settings. Native editing and system shortcuts are reserved.", rune::Chord::parse("mod+comma").unwrap().label())).font(p.label).color(p.muted).wrap())
}

pub fn panel(state: &State, p: &Palette) -> Option<El> {
    if !state.settings_open {
        return None;
    }
    let mut navigation = col()
        .w(145.0_f32.max(p.label.size * 8.0 + 12.0))
        .h_full()
        .gap(6.0)
        .pr(12.0);
    for (page, key, label) in [
        (Page::Appearance, "settings-appearance", "Appearance"),
        (Page::Buttons, "settings-buttons", "Buttons"),
        (Page::Shortcuts, "settings-shortcuts", "Shortcuts"),
    ] {
        navigation = navigation.child(
            button(key, label, p)
                .w_full()
                .fill(if state.settings_page == page {
                    p.accent.alpha(0.12)
                } else {
                    p.button_fill()
                })
                .on_press_tx(move |s: &mut State, tx| {
                    s.settings_page = page;
                    s.recording = None;
                    tx.blur();
                }),
        );
    }
    let content = match state.settings_page {
        Page::Appearance => appearance(state, p),
        Page::Buttons => buttons(state, p),
        Page::Shortcuts => shortcuts(state, p),
    };
    Some(el().key("settings-scrim")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .padding(20.0).fill(Color::BLACK.alpha(0.6)).center()
        .child(col().key("settings-panel").w_full().max_w(740.0).h_full().max_h(590.0)
            .padding(22.0).gap(18.0).radius(12.0).fill(p.panel).border(1.0, p.line)
            .role("dialog").label("Settings")
            .modal(true, |s: &mut State| {
                if s.recording.take().is_none() { s.settings_open = false; }
            })
            .modal_click_outside(false)
            .on_key(|s: &mut State, event, tx| {
                if s.recording.is_some() { record_key(s, event, tx) }
                else if event.key == Key::Escape { s.close_settings(tx); true }
                else { false }
            })
            .child(row().w_full().center_y()
                .child(text("Settings").font(p.title).color(p.ink))
                .child(el().grow(1.0))
                .child(button("settings-close", "Done", p).on_press_tx(State::close_settings)))
            .child(row().w_full().grow(1.0).min_h(0.0).gap(16.0)
                .child(navigation)
                .child(col().key("settings-content").grow(1.0).min_w(0.0).h_full()
                    .scroll(rune::ScrollSpec { stick_bottom: false }).child(content)))
            .children(state.settings_error.as_ref().map(|error| text(error).key("settings-error").font(p.label).color(p.accent).wrap()))
            .child(text(if state.prefs.is_persistent() { "Saved automatically on this device · Running shells are never restarted." } else { "Preview · Changes are kept in memory only." }).font(p.label).color(p.muted).wrap())
        ))
}
