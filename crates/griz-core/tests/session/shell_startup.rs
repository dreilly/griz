use super::*;
use std::os::unix::fs::PermissionsExt;

fn finder_like_zsh() -> Service {
    Service::start_with("/bin/zsh", |home, command| {
        let tools = home.join("tools with spaces");
        fs::create_dir(&tools).unwrap();
        for name in ["starship", "fzf", "nvim"] {
            let path = tools.join(name);
            fs::write(&path, format!("#!/bin/sh\nprintf 'fixture-{name}'\n")).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(
            home.join(".zprofile"),
            "export PATH=\"$HOME/tools with spaces:$PATH\"\nexport GRIZ_PROFILE_LOADED=yes\n",
        )
        .unwrap();
        fs::write(
            home.join(".zshrc"),
            "PROMPT='griz$ '\nexport GRIZ_RC_TOOLS=\"$(starship)-$(fzf)-$(nvim)\"\n",
        )
        .unwrap();
        // Model a GUI launched outside an already-configured terminal. Do not
        // inherit the developer's Homebrew PATH or read their real dotfiles.
        command
            .env("HOME", home)
            .env("ZDOTDIR", home)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .env_remove("SHELL")
            .env_remove("GRIZ_PROFILE_LOADED")
            .env_remove("GRIZ_RC_TOOLS");
    })
}

fn assert_login_startup(stream: &mut UnixStream, serial: u64, terminal_id: u64) {
    input(
        stream,
        serial,
        terminal_id,
        b"printf 'startup:<%s>:<%s>:<%s>\\n' \"$options[login]\" \"$GRIZ_PROFILE_LOADED\" \"$GRIZ_RC_TOOLS\"\r",
    );
    until(stream, |event| {
        contains(
            event,
            "startup:<on>:<yes>:<fixture-starship-fixture-fzf-fixture-nvim>",
        )
    });
}

#[test]
fn login_profile_sets_tool_paths_before_interactive_rc_in_every_new_shell() {
    let service = finder_like_zsh();
    let mut stream = service.attach(120, 30);
    let initial = next_view(&mut stream);
    let first = initial.active_terminal.unwrap();
    assert_login_startup(&mut stream, 1, first);

    command(
        &mut stream,
        2,
        CoreCommand::CreateTerminal {
            workspace_id: initial.active_workspace,
            cwd: None,
        },
    );
    let second = ack(&mut stream, 2).active_terminal.unwrap();
    assert_ne!(second, first);
    assert_login_startup(&mut stream, 3, second);

    command(
        &mut stream,
        4,
        CoreCommand::SplitTerminal {
            terminal_id: second,
            direction: SplitDirection::Vertical,
        },
    );
    let third = ack(&mut stream, 4).active_terminal.unwrap();
    assert_ne!(third, second);
    assert_login_startup(&mut stream, 5, third);

    input(&mut stream, 6, third, b"exit\r");
    until(&mut stream, |event| {
        matches!(event, Event::View { state } if state.workspaces.iter()
            .flat_map(|w| &w.terminals).any(|t| t.id == third && t.exited.is_some()))
    });
    command(
        &mut stream,
        7,
        CoreCommand::StartShell { terminal_id: third },
    );
    ack(&mut stream, 7);
    assert_login_startup(&mut stream, 8, third);
}
