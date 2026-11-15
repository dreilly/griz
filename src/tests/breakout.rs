use super::*;
use rune::Key;

#[test]
fn typing_break_reveals_and_opens_the_secret_game_without_touching_shells() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    command_palette_tests::open(&mut rt, true);
    assert!(rt.find("palette-breakout").is_none());
    commands(&outgoing);

    command_palette_tests::press(&mut rt, Key::Text("break".into()));
    command_palette_tests::settle(&mut rt);
    assert!(rt.find("palette-breakout").is_some());
    command_palette_tests::press(&mut rt, Key::Enter);
    command_palette_tests::settle(&mut rt);

    assert!(!rt.state.palette.is_open());
    assert!(rt.state.breakout.is_some());
    assert!(rt.find("breakout-workspace").is_some());
    assert!(rt.find("breakout-board").is_some());
    assert!(rt.find("breakout-brick-0").is_some());
    assert!(
        !commands(&outgoing)
            .iter()
            .any(|(_, command)| terminal_input(command))
    );
}

#[test]
fn game_controls_move_launch_animate_and_escape_back_to_work() {
    let (mut state, _) = mock();
    state.breakout = Some(breakout::Game::default());
    let mut rt = make_runtime(state, (900.0, 640.0));
    let start_paddle = rt.state.breakout.as_ref().unwrap().paddle_x;

    command_palette_tests::press(&mut rt, Key::Right);
    assert!(rt.state.breakout.as_ref().unwrap().paddle_x > start_paddle);
    command_palette_tests::press(&mut rt, Key::Text(" ".into()));
    assert_eq!(
        rt.state.breakout.as_ref().unwrap().phase,
        breakout::Phase::Playing
    );
    let start_ball = rt.state.breakout.as_ref().unwrap().ball;

    rt.update(|state, tx| {
        breakout::poll(state, 0.0, tx);
    });
    rt.update(|state, tx| {
        breakout::poll(state, 1.0 / 30.0, tx);
    });
    rt.frame(0.0);
    assert_ne!(rt.state.breakout.as_ref().unwrap().ball, start_ball);

    command_palette_tests::press(&mut rt, Key::Escape);
    assert!(rt.state.breakout.is_none());
    assert!(rt.find("breakout-workspace").is_none());
}
