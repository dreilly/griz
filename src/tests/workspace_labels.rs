use super::*;

#[test]
fn automatic_workspace_labels_and_hover_paths_update_with_the_snapshot() {
    let (mut state, outgoing) = mock();
    let mut initial = snapshot();
    initial.workspaces[0].name = "code".into();
    initial.workspaces[0].custom_name = None;
    initial.workspaces[0].directory = "~/code".into();
    state.apply(initial);
    let mut rt = make_runtime(state, (1000.0, 680.0));
    let (x, y) = rt.find("workspace-1").unwrap();
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    rt.frame(1.0);
    assert_eq!(rt.help_tooltip().map(|(text, _)| text), Some("~/code"));

    let mut updated = snapshot();
    updated.workspaces[0].name = "griz".into();
    updated.workspaces[0].custom_name = None;
    updated.workspaces[0].directory = "~/code/griz".into();
    rt.update(|s, _| s.apply(updated));
    rt.frame(0.0);
    rt.frame(1.0);
    assert!(
        rt.elements()
            .iter()
            .any(|element| element.text.as_deref() == Some("griz"))
    );
    assert!(
        !rt.elements()
            .iter()
            .any(|element| element.text.as_deref() == Some("code"))
    );
    assert_eq!(rt.help_tooltip().map(|(text, _)| text), Some("~/code/griz"));

    // Inactive and custom-named workspaces expose the selected terminal's path too.
    let (x, y) = rt.find("workspace-2").unwrap();
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    rt.frame(1.0);
    assert_eq!(rt.help_tooltip().map(|(text, _)| text), Some("/tmp"));
    assert_eq!(rt.state.snapshot.as_ref().unwrap().active_workspace, 1);
    assert!(
        !commands(&outgoing)
            .iter()
            .any(|(_, command)| matches!(command, Command::SelectWorkspace { .. }))
    );
}

#[test]
fn opening_rename_for_an_automatic_label_keeps_the_name_blank_to_preserve_tracking() {
    let (mut state, outgoing) = mock();
    let mut initial = snapshot();
    initial.workspaces[1].name = "griz".into();
    initial.workspaces[1].custom_name = None;
    initial.workspaces[1].directory = "~/code/griz".into();
    state.apply(initial);
    let mut rt = make_runtime(state, (1000.0, 680.0));
    workspace_menu(&mut rt, 2);
    pick_workspace_menu(&mut rt, "rename");
    assert!(
        rt.state.draft.text.is_empty(),
        "opening Rename must not pin the automatic basename as a custom name"
    );
    commands(&outgoing);
    click(&mut rt, "dialog-submit");
    assert!(commands(&outgoing).iter().any(|(_, command)| matches!(
        command, Command::RenameWorkspace { workspace_id: 2, name } if name.is_empty()
    )));
}
