use super::*;

#[test]
fn silent_input_and_input_to_hidden_terminals_still_get_acknowledged() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let first = initial.active_terminal.unwrap();
    input(
        &mut stream,
        1,
        first,
        b"stty -echo; printf '\\033[2J\\033[Hsilent-%s' ready; read value; printf 'read-%s' done\r",
    );
    until(&mut stream, |event| contains(event, "silent-ready"));
    input(&mut stream, 2, first, b"x");
    let silent = ack(&mut stream, 2);
    assert!(text(&silent.terminal.as_ref().unwrap().screen).contains("silent-ready"));
    assert!(!text(&silent.terminal.as_ref().unwrap().screen).contains('x'));
    input(&mut stream, 3, first, b"\r");
    until(&mut stream, |event| contains(event, "read-done"));

    command(
        &mut stream,
        4,
        CoreCommand::CreateTerminal {
            workspace_id: initial.active_workspace,
            cwd: None,
        },
    );
    let second = ack(&mut stream, 4).active_terminal.unwrap();
    input(&mut stream, 5, first, b"x");
    let hidden = ack(&mut stream, 5);
    assert_eq!(hidden.active_terminal, Some(second));
    assert_eq!(hidden.terminals.len(), 1);
}

#[test]
fn a_selection_after_input_acknowledges_both_without_retargeting_the_key() {
    let service = Service::start();
    let mut stream = service.attach(80, 24);
    let initial = next_view(&mut stream);
    let first = initial.active_terminal.unwrap();
    command(
        &mut stream,
        1,
        CoreCommand::CreateTerminal {
            workspace_id: initial.active_workspace,
            cwd: None,
        },
    );
    let second = ack(&mut stream, 1).active_terminal.unwrap();
    input(&mut stream, 2, second, b"printf 'key-%s\\n' routed\r");
    command(
        &mut stream,
        3,
        CoreCommand::SelectTerminal { terminal_id: first },
    );
    let selected = ack(&mut stream, 3);
    assert_eq!(selected.active_terminal, Some(first));
    assert!(!text(&selected.terminal.unwrap().screen).contains("key-routed"));
    command(
        &mut stream,
        4,
        CoreCommand::SelectTerminal {
            terminal_id: second,
        },
    );
    until(&mut stream, |event| contains(event, "key-routed"));
}
