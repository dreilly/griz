//! Opt-in CPU-side GUI measurements; does not include GPU submission or display scanout.
//! Run: cargo test -p griz input_latency -- --ignored --nocapture --test-threads=1
use super::*;
use rune::Key;
use std::time::Instant;

#[test]
fn sending_a_key_does_not_rebuild_an_unchanged_screen_but_echo_does() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    rt.frame(0.0);
    commands(&outgoing);
    let before = rt.stats.commits;
    rt.key(Key::Text("x".into()).into());
    rt.frame(0.0);
    assert_eq!(
        rt.stats.commits, before,
        "sending input must not rebuild the old screen"
    );
    assert!(matches!(
        commands(&outgoing).as_slice(),
        [(_, Command::Key { terminal_id: 1, .. })]
    ));
    let mut snapshot = rt.state.snapshot.clone().unwrap();
    snapshot.terminal.as_mut().unwrap().screen.cells[0][0].text = "x".into();
    rt.update(|state, tx| {
        state.apply(snapshot);
        tx.instant();
    });
    rt.frame(0.0);
    assert!(rt.stats.commits > before);
    assert!(
        rt.elements()
            .iter()
            .any(|el| el.text.as_deref() == Some("x"))
    );
}

#[test]
fn input_errors_and_ime_composition_still_rebuild_visible_state() {
    let (state, outgoing) = mock();
    let mut rt = make_runtime(state, (1100.0, 720.0));
    rt.frame(0.0);
    rt.update(|state, _| state.online = false);
    rt.frame(0.0);
    let before = rt.stats.commits;
    rt.key(Key::Text("x".into()).into());
    rt.frame(0.0);
    assert!(rt.stats.commits > before);
    assert!(
        rt.elements()
            .iter()
            .any(|el| el.text.as_deref() == Some("Not connected. Click Reconnect."))
    );
    commands(&outgoing);
    rt.update(|state, tx| {
        state.online = true;
        on_ime(state, &Ime::Preedit("か".into(), None), tx);
    });
    rt.frame(0.0);
    assert!(
        rt.elements()
            .iter()
            .any(|el| el.text.as_deref() == Some("か"))
    );
    rt.update(|state, tx| {
        on_ime(state, &Ime::Commit("家".into()), tx);
    });
    rt.frame(0.0);
    assert!(
        !rt.elements()
            .iter()
            .any(|el| el.text.as_deref() == Some("か"))
    );
    assert!(
        matches!(commands(&outgoing).as_slice(), [(_, Command::Input { data, .. })] if data == "家".as_bytes())
    );
}

fn report(label: &str, mut samples: Vec<f64>) {
    samples.sort_by(f64::total_cmp);
    println!(
        "{label}: median={:.3}ms p95={:.3}ms",
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100]
    );
}

#[test]
#[ignore = "manual CPU timing benchmark, not a correctness test"]
fn input_latency_gui_cpu() {
    for dense in [false, true] {
        let (mut state, outgoing) = mock();
        let mut terminal = griz_vt::Terminal::new(120, 36).unwrap();
        if dense {
            for _ in 0..35 {
                terminal.write(b"abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz0123456789abcdefghijklmnopqrstuvwxyz0123456789\r\n");
            }
        }
        state
            .snapshot
            .as_mut()
            .unwrap()
            .terminal
            .as_mut()
            .unwrap()
            .screen = terminal.screen().unwrap();
        let mut rt = make_runtime(state, (1280.0, 900.0));
        rt.frame(0.0);
        let mut dispatch = Vec::new();
        let mut apply = Vec::new();
        for index in 0..70 {
            commands(&outgoing);
            let started = Instant::now();
            rt.key(Key::Text("x".into()).into());
            rt.frame(0.0);
            let input_ms = started.elapsed().as_secs_f64() * 1000.0;
            let mut snapshot = rt.state.snapshot.clone().unwrap();
            let screen = &mut snapshot.terminal.as_mut().unwrap().screen;
            screen.cells[35][index % 100].text = if index % 2 == 0 { "x" } else { "y" }.into();
            screen.cursor_x = (index % 100 + 1) as u16;
            let started = Instant::now();
            rt.update(|state, tx| {
                state.apply(snapshot);
                tx.instant();
            });
            rt.frame(0.0);
            let apply_ms = started.elapsed().as_secs_f64() * 1000.0;
            if index >= 10 {
                dispatch.push(input_ms);
                apply.push(apply_ms);
            }
        }
        report(
            &format!("GUI key dispatch + logical frame (dense={dense})"),
            dispatch,
        );
        report(&format!("GUI apply + logical frame (dense={dense})"), apply);
    }
}
