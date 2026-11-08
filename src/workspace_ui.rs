//! Workspace-specific chrome and the right-click actions, including Rune-style accent swatches.
use crate::{Command, Dialog, State, ui::Palette};
use griz_protocol::{WorkspaceAccent, WorkspaceId};
use rune::{Color, Cx, El, col, el, row, text, tok};

pub(crate) fn accent_index(accent: WorkspaceAccent) -> usize {
    match accent {
        WorkspaceAccent::Blue => 0,
        WorkspaceAccent::Purple => 1,
        WorkspaceAccent::Pink => 2,
        WorkspaceAccent::Orange => 3,
        WorkspaceAccent::Green => 4,
        WorkspaceAccent::Graphite => 5,
    }
}

pub(crate) fn accent_color(accent: WorkspaceAccent) -> Color {
    // Both app themes are dark. Use the same six OKLCH colors as Rune's Settings example.
    tok::accent(accent_index(accent), true)
}

pub(crate) fn menu(
    state: &State,
    workspace_id: WorkspaceId,
    position: (f32, f32),
    p: &Palette,
    cx: &Cx<State>,
) -> Option<El> {
    if !state.ready() || state.settings_open {
        return None;
    }
    let workspace = state
        .snapshot
        .as_ref()?
        .workspaces
        .iter()
        .find(|workspace| workspace.id == workspace_id)?;
    let selected = workspace.accent;
    let accent = accent_color(selected);
    let mut swatches = row()
        .key("workspace-accent-colors")
        .role("radiogroup")
        .label("Workspace accent color")
        .focus_group()
        .gap(8.0);
    for color in WorkspaceAccent::ALL {
        let name = tok::ACCENT_NAMES[accent_index(color)];
        let fill = accent_color(color);
        swatches = swatches.child(
            el().key(format!("workspace-accent-{name}"))
                .role("radio")
                .label(format!("{name} workspace accent"))
                .checked(color == selected)
                .focus_selected(color == selected)
                .focusable(true)
                .size(32.0, 32.0)
                .center()
                .radius(16.0)
                .border(
                    2.0,
                    if color == selected {
                        fill
                    } else {
                        Color::TRANSPARENT
                    },
                )
                .hover(p.ink.alpha(0.08))
                .focus_ring(2.0, 2.0, fill)
                .child(el().size(20.0, 20.0).radius(10.0).fill(fill))
                .on_press_tx(move |s: &mut State, tx| {
                    if s.ready()
                        && s.snapshot.as_ref().is_some_and(|snapshot| {
                            snapshot
                                .workspaces
                                .iter()
                                .any(|workspace| workspace.id == workspace_id)
                        })
                    {
                        s.dialog = None;
                        s.change(Command::SetWorkspaceAccent {
                            workspace_id,
                            accent: color,
                        });
                        tx.blur();
                    }
                }),
        );
    }
    let [width, height] = cx.layout_size("root").unwrap_or([1100.0, 720.0]);
    let menu_width = (260.0_f32.max(p.label.size * 14.0 + 24.0)).min((width - 16.0).max(1.0));
    // These are the five children's heights, four gaps, and the panel's padding.
    let menu_height = p.label.line_px().ceil() + 32.0 + 1.0 + 2.0 * p.control_height() + 64.0;
    let left = position.0.clamp(8.0, (width - menu_width - 8.0).max(8.0));
    let top = position.1.clamp(8.0, (height - menu_height - 8.0).max(8.0));
    Some(
        col()
            .key("workspace-context-menu")
            .role("dialog")
            .label(format!("Actions for {}", workspace.name))
            .absolute(left, top)
            .w(menu_width)
            .padding(12.0)
            .gap(10.0)
            .radius(8.0)
            .fill(p.panel)
            .border(1.0, p.chrome_line)
            .shadow(6.0, 24.0, Color::BLACK.alpha(0.35))
            .modal(true, |s: &mut State| s.dialog = None)
            .child(text("Accent color").font(p.label).color(p.muted))
            .child(swatches)
            .child(el().h(1.0).w_full().fill(p.chrome_line))
            .child(
                crate::ui::button("workspace-menu-rename", "Rename workspace…", p)
                    .w_full()
                    .focus_ring(2.0, 2.0, accent)
                    .on_press_tx(move |s: &mut State, tx| {
                        if s.snapshot.as_ref().is_some_and(|snapshot| {
                            snapshot
                                .workspaces
                                .iter()
                                .any(|workspace| workspace.id == workspace_id)
                        }) {
                            s.open_dialog(Dialog::RenameWorkspace(workspace_id), tx);
                        }
                    }),
            )
            .child(
                crate::ui::button("workspace-menu-close", "Close workspace…", p)
                    .w_full()
                    .focus_ring(2.0, 2.0, accent)
                    .on_press_tx(move |s: &mut State, tx| {
                        if let Some(workspace) = s.snapshot.as_ref().and_then(|snapshot| {
                            snapshot
                                .workspaces
                                .iter()
                                .find(|workspace| workspace.id == workspace_id)
                        }) {
                            s.open_dialog(
                                Dialog::CloseWorkspace(workspace_id, workspace.name.clone()),
                                tx,
                            );
                        }
                    }),
            ),
    )
}
