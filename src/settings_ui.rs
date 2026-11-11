//! Settings presentation; the typed model and persistence stay in settings.rs.
use crate::{
    State,
    settings::{self, Action, Font, Settings, Theme},
    ui::{Palette, button},
};
use griz_protocol::WorkspaceAccent;
use rune::{Color, El, Family, Key, KeyEvent, TextStyle, Tx, col, el, row, text, text_input};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Appearance,
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
        .gap(8.0)
        .child(
            text(title)
                .font(TextStyle::new(p.title.size * 1.35, 600.0))
                .color(p.ink),
        )
        .child(text(hint).font(p.input).color(p.muted).wrap())
}

fn section(title: &str, p: &Palette) -> El {
    text(title).font(p.section).color(p.muted)
}

fn divider(p: &Palette) -> El {
    el().w_full().h(1.0).fill(p.line)
}

fn setting_label(title: &str, hint: &str, p: &Palette) -> El {
    col()
        .grow(1.0)
        .min_w(160.0)
        .gap(5.0)
        .child(text(title).font(p.label).color(p.ink))
        .child(text(hint).font(p.input).color(p.muted).wrap())
}

fn theme_card(index: usize, theme: Theme, state: &State, p: &Palette) -> El {
    let selected = state.settings.theme == theme;
    row()
        .key(format!("settings-theme-{index}"))
        .role("radio")
        .label(format!("Use {} theme", theme.name()))
        .checked(selected)
        .focus_selected(selected)
        .focusable(true)
        .w(176.0)
        .padding(12.0)
        .gap(10.0)
        .center_y()
        .radius(6.0)
        .fill(crate::terminal_view::background_for(theme))
        .border(1.0, if selected { p.accent } else { p.line })
        .hover(p.ink.alpha(0.06))
        .focus_ring(2.0, 2.0, p.accent)
        .child(
            text(">_")
                .font(p.input.family(Family::SystemMono))
                .color(if selected { p.accent } else { p.muted }),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(4.0)
                .child(text(theme.name()).font(p.label).color(p.ink).wrap())
                .child(
                    text(if theme == Theme::Dark {
                        "Charcoal"
                    } else {
                        "Deep blue"
                    })
                    .font(p.input)
                    .color(p.muted)
                    .wrap(),
                ),
        )
        .on_press(move |s: &mut State| {
            s.settings.theme = theme;
            s.settings_changed();
        })
}

fn workspace_accent(state: &State, p: &Palette) -> El {
    let selected = state.settings.default_workspace_accent;
    let mut swatches = row()
        .key("settings-accent-colors")
        .w_full()
        .flex_wrap()
        .gap(8.0)
        .role("radiogroup")
        .label("Default workspace accent")
        .focus_group();
    for accent in WorkspaceAccent::ALL {
        let name = rune::tok::ACCENT_NAMES[crate::workspace_ui::accent_index(accent)];
        let color = crate::workspace_ui::accent_color(accent);
        swatches = swatches.child(
            el().key(format!("settings-accent-{name}"))
                .role("radio")
                .label(format!("{name} default workspace accent"))
                .checked(accent == selected)
                .focus_selected(accent == selected)
                .focusable(true)
                .help(name)
                .size(36.0, 32.0)
                .center()
                .radius(5.0)
                .fill(color.alpha(0.16))
                .border(
                    1.0,
                    if accent == selected {
                        color
                    } else {
                        Color::TRANSPARENT
                    },
                )
                .hover(color.alpha(0.25))
                .focus_ring(2.0, 2.0, color)
                .child(
                    el().size(16.0, 16.0)
                        .radius(3.0)
                        .fill(color)
                        .center()
                        .children(
                            (accent == selected)
                                .then(|| text("✓").font(TextStyle::new(12.0, 600.0)).color(p.bg)),
                        ),
                )
                .on_press(move |s: &mut State| {
                    s.settings.default_workspace_accent = accent;
                    s.settings_changed();
                }),
        );
    }
    col()
        .w_full()
        .gap(12.0)
        .child(
            row()
                .w_full()
                .center_y()
                .flex_wrap()
                .gap(8.0)
                .child(text("Default workspace accent").font(p.label).color(p.ink))
                .child(el().grow(1.0))
                .child(
                    text(rune::tok::ACCENT_NAMES[crate::workspace_ui::accent_index(selected)])
                        .key("settings-accent-name")
                        .font(p.input)
                        .color(crate::workspace_ui::accent_color(selected)),
                ),
        )
        .child(swatches)
        .child(
            text("For new workspaces. Existing workspace colors stay unchanged.")
                .font(p.input)
                .color(p.muted)
                .wrap(),
        )
}

fn appearance(state: &State, p: &Palette) -> El {
    let mut fonts = row()
        .min_w(0.0)
        .shrink()
        .flex_wrap()
        .gap(4.0)
        .padding(4.0)
        .radius(8.0)
        .fill(p.bg)
        .border(1.0, p.line);
    for (index, font) in Font::ALL.into_iter().enumerate() {
        let selected = state.settings.font == font;
        fonts = fonts.child(
            button(format!("settings-font-{index}"), font.name(), p)
                .fill(if selected {
                    p.ink.alpha(0.12)
                } else {
                    Color::TRANSPARENT
                })
                .border(
                    1.0,
                    if selected {
                        p.ink.alpha(0.12)
                    } else {
                        Color::TRANSPARENT
                    },
                )
                .on_press(move |s: &mut State| {
                    s.settings.font = font;
                    s.settings_changed();
                }),
        );
    }
    let mut themes = row()
        .w_full()
        .flex_wrap()
        .gap(10.0)
        .role("radiogroup")
        .label("Terminal theme")
        .focus_group();
    for (index, theme) in Theme::ALL.into_iter().enumerate() {
        themes = themes.child(theme_card(index, theme, state, p));
    }
    let terminal_font =
        TextStyle::new(state.settings.font_size, 400.0).family(state.settings.font.family());
    col()
        .w_full()
        .gap(18.0)
        .child(heading(
            "Appearance",
            "A little less chrome. A little more you.",
            p,
        ))
        .child(
            col()
                .w_full()
                .gap(12.0)
                .child(section("SURFACE", p))
                .child(themes),
        )
        .child(workspace_accent(state, p))
        .child(divider(p))
        .child(
            col()
                .w_full()
                .gap(18.0)
                .child(section("TYPE & SIZE", p))
                .child(
                    row()
                        .w_full()
                        .center_y()
                        .flex_wrap()
                        .gap(14.0)
                        .child(setting_label(
                            "Terminal font",
                            "A monospace face for your shells.",
                            p,
                        ))
                        .child(fonts),
                )
                .child(
                    row()
                        .w_full()
                        .center_y()
                        .flex_wrap()
                        .gap(14.0)
                        .child(setting_label(
                            "App font size",
                            "Applies throughout the app.",
                            p,
                        ))
                        .child(
                            row()
                                .key("settings-size-control")
                                .gap(12.0)
                                .center_y()
                                .child(
                                    button("settings-size-down", "−", p)
                                        .label("Decrease font size")
                                        .disabled(state.settings.font_size <= 10.0)
                                        .on_press(|s: &mut State| {
                                            s.change_font_size(-1.0);
                                        }),
                                )
                                .child(
                                    text(format!("{} pt", state.settings.font_size))
                                        .font(p.input)
                                        .color(p.ink),
                                )
                                .child(
                                    button("settings-size-up", "+", p)
                                        .label("Increase font size")
                                        .disabled(state.settings.font_size >= 28.0)
                                        .on_press(|s: &mut State| {
                                            s.change_font_size(1.0);
                                        }),
                                ),
                        ),
                ),
        )
        .child(
            col()
                .key("settings-preview")
                .w_full()
                .radius(8.0)
                .border(1.0, p.line)
                .fill(crate::terminal_view::background_for(state.settings.theme))
                .child(
                    row()
                        .w_full()
                        .px(16.0)
                        .py(9.0)
                        .center_y()
                        .border_sides([0.0, 0.0, 1.0, 0.0], p.line)
                        .child(
                            text("workspace / terminal")
                                .font(p.input.family(Family::SystemMono))
                                .color(p.muted),
                        )
                        .child(el().grow(1.0))
                        .child(
                            text(state.settings.font.name())
                                .font(p.input)
                                .color(p.muted),
                        ),
                )
                .child(
                    col()
                        .w_full()
                        .padding(16.0)
                        .gap(8.0)
                        .border_sides(
                            [0.0, 0.0, 0.0, 3.0],
                            crate::workspace_ui::accent_color(
                                state.settings.default_workspace_accent,
                            ),
                        )
                        .child(
                            text("griz ~/workspace > echo hello")
                                .font(terminal_font)
                                .color(p.ink)
                                .wrap(),
                        )
                        .child(text("hello, world").font(terminal_font).color(
                            crate::workspace_ui::accent_color(
                                state.settings.default_workspace_accent,
                            ),
                        )),
                ),
        )
        .child(
            row().w_full().child(
                button("settings-appearance-reset", "Restore defaults", p)
                    .fill(Color::TRANSPARENT)
                    .border(1.0, p.line)
                    .on_press(|s: &mut State| {
                        let defaults = Settings::default();
                        s.settings.font = defaults.font;
                        s.settings.font_size = defaults.font_size;
                        s.settings.theme = defaults.theme;
                        s.settings.default_workspace_accent = defaults.default_workspace_accent;
                        s.settings_changed();
                    }),
            ),
        )
}

fn shortcuts(state: &State, p: &Palette) -> El {
    let mut list = col().w_full();
    let search = state.shortcut_search.text.to_lowercase();
    let mut count = 0;
    for action in Action::ALL {
        if !action.label().to_lowercase().contains(&search) && !action.id().contains(&search) {
            continue;
        }
        count += 1;
        let recording = state.recording == Some(action);
        list = list.child(
            row()
                .w_full()
                .center_y()
                .flex_wrap()
                .gap(10.0)
                .py(10.0)
                .border_sides([0.0, 0.0, 1.0, 0.0], p.line)
                .child(
                    text(action.label())
                        .font(p.input)
                        .color(p.ink)
                        .grow(1.0)
                        .min_w(160.0)
                        .wrap(),
                )
                .child(
                    row()
                        .flex_wrap()
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
                            .min_w(104.0)
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
    col()
        .w_full()
        .gap(14.0)
        .child(heading(
            "Keyboard shortcuts",
            "Click a shortcut to record a new combination. Press Escape to cancel.",
            p,
        ))
        .child(
            text_input(&state.shortcut_search, p.input)
                .key("settings-shortcut-search")
                .label("Search shortcuts")
                .single_line()
                .w_full()
                .padding(10.0)
                .radius(7.0)
                .fill(p.bg)
                .border(1.0, p.line)
                .color(p.ink)
                .placeholder("Search actions…", p.muted)
                .disabled(state.recording.is_some())
                .on_edit(|s: &mut State| &mut s.shortcut_search),
        )
        .child(list)
        .child(
            text(format!(
                "{} always opens Settings. Native editing and system shortcuts are reserved.",
                rune::Chord::parse("mod+comma").unwrap().label()
            ))
            .font(p.label)
            .color(p.muted)
            .wrap(),
        )
}

pub fn panel(state: &State, p: &Palette) -> Option<El> {
    if !state.settings_open {
        return None;
    }
    let mut navigation = col()
        .key("settings-navigation")
        .w(184.0)
        .h_full()
        .padding(12.0)
        .gap(6.0)
        .fill(p.bg)
        .border_sides([0.0, 1.0, 0.0, 0.0], p.line)
        .child(
            text("griz >_")
                .font(TextStyle::new(p.title.size * 1.3, 600.0).family(Family::SystemMono))
                .color(p.accent)
                .px(12.0)
                .pt(16.0),
        )
        .child(
            text("Settings")
                .font(p.title)
                .color(p.ink)
                .px(12.0)
                .py(14.0),
        );
    for (page, key, label, number) in [
        (Page::Appearance, "settings-appearance", "Appearance", "01"),
        (Page::Shortcuts, "settings-shortcuts", "Shortcuts", "02"),
    ] {
        let selected = state.settings_page == page;
        navigation = navigation.child(
            row()
                .key(key)
                .w_full()
                .min_h(40.0_f32.max(p.control_height() + 8.0))
                .px(12.0)
                .gap(10.0)
                .center_y()
                .radius(3.0)
                .border_sides(
                    [0.0, 0.0, 0.0, 2.0],
                    if selected {
                        p.accent
                    } else {
                        Color::TRANSPARENT
                    },
                )
                .role("button")
                .label(label)
                .focusable(true)
                .fill(if selected {
                    p.ink.alpha(0.07)
                } else {
                    Color::TRANSPARENT
                })
                .hover(p.ink.alpha(0.06))
                .focus_ring(2.0, 2.0, p.accent)
                .child(
                    text(number)
                        .font(p.section.family(Family::SystemMono))
                        .color(if selected { p.accent } else { p.muted }),
                )
                .child(
                    text(label)
                        .font(p.label)
                        .color(if selected { p.ink } else { p.muted })
                        .grow(1.0)
                        .min_w(0.0)
                        .truncate(),
                )
                .on_press_tx(move |s: &mut State, tx| {
                    s.settings_page = page;
                    s.recording = None;
                    tx.blur();
                    tx.scroll_to("settings-content", 0.0, 0.0);
                }),
        );
    }
    navigation = navigation.child(el().grow(1.0)).child(
        col()
            .key("settings-save-status")
            .w_full()
            .padding(12.0)
            .gap(6.0)
            .child(
                text(if state.prefs.is_persistent() {
                    "Saved automatically"
                } else {
                    "Preview mode"
                })
                .font(p.section)
                .color(p.muted)
                .wrap(),
            )
            .child(
                text(if state.prefs.is_persistent() {
                    "On this device"
                } else {
                    "Not saved to disk"
                })
                .font(p.input)
                .color(p.muted)
                .wrap(),
            ),
    );
    let content = match state.settings_page {
        Page::Appearance => appearance(state, p),
        Page::Shortcuts => shortcuts(state, p),
    };
    let surface = crate::terminal_view::color(if state.settings.theme == Theme::Midnight {
        [17, 24, 35]
    } else {
        [24, 26, 28]
    });
    Some(
        el().key("settings-scrim")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .padding(20.0)
            .fill(Color::BLACK.alpha(0.6))
            .center()
            .child(
                row()
                    .key("settings-panel")
                    .w_full()
                    .max_w(960.0)
                    .h_full()
                    .max_h(720.0)
                    .radius(12.0)
                    .clip_rounded(12.0)
                    .fill(surface)
                    .border(1.0, p.line)
                    .role("dialog")
                    .label("Settings")
                    .modal(true, |s: &mut State| {
                        if s.recording.take().is_none() {
                            s.settings_open = false;
                        }
                    })
                    .modal_click_outside(false)
                    .on_key(|s: &mut State, event, tx| {
                        if s.recording.is_some() {
                            record_key(s, event, tx)
                        } else if event.key == Key::Escape {
                            s.close_settings(tx);
                            true
                        } else {
                            false
                        }
                    })
                    .child(navigation)
                    .child(
                        col()
                            .grow(1.0)
                            .min_w(0.0)
                            .h_full()
                            .child(
                                row()
                                    .w_full()
                                    .px(24.0)
                                    .py(12.0)
                                    .center_y()
                                    .child(
                                        text("griz / settings")
                                            .font(p.input.family(Family::SystemMono))
                                            .color(p.muted)
                                            .grow(1.0)
                                            .min_w(0.0)
                                            .truncate(),
                                    )
                                    .child(
                                        button("settings-close", "Done", p)
                                            .fill(Color::TRANSPARENT)
                                            .border(1.0, p.line)
                                            .on_press_tx(State::close_settings),
                                    ),
                            )
                            .child(
                                col()
                                    .key("settings-content")
                                    .w_full()
                                    .grow(1.0)
                                    .min_h(0.0)
                                    .scroll(rune::ScrollSpec {
                                        stick_bottom: false,
                                    })
                                    .child(
                                        col()
                                            .w_full()
                                            .padding(24.0)
                                            .pt(8.0)
                                            .gap(18.0)
                                            .child(content)
                                            .children(state.settings_error.as_ref().map(|error| {
                                                text(error)
                                                    .key("settings-error")
                                                    .font(p.input)
                                                    .color(p.accent)
                                                    .wrap()
                                            })),
                                    ),
                            ),
                    ),
            ),
    )
}
