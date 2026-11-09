use super::*;
use griz_protocol::{AgentStatus, ProgramStatusKind};

fn report(vt: &mut Terminal, body: &str) {
    vt.write(format!("\x1b]7501;{body}\x1b\\").as_bytes());
}

#[test]
fn capability_reply_precedes_da_and_never_echoes_report_text() {
    let mut vt = Terminal::new(80, 24).unwrap();
    vt.write(b"\x1b]7501;?\x1b\\\x1b[c");
    let replies = String::from_utf8(vt.take_responses()).unwrap();
    let expected =
        "\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\";
    assert!(replies.starts_with(expected), "{replies:?}");
    assert!(replies[expected.len()..].starts_with("\x1b[?"));
    assert!(vt.program_status().is_empty());
    report(&mut vt, "state=blocked:app=tool:msg=QXBwbHk/");
    assert!(vt.take_responses().is_empty());
    assert_eq!(vt.program_status()[0].message, "Apply?");
}

#[test]
fn arbitrary_pty_chunking_bel_and_st_copy_every_field() {
    let mut vt = Terminal::new(80, 24).unwrap();
    let sequence = b"\x1b]7501;state=blocked:kind=permission:app=terraform:id=build/test:progress=42:title=VGVzdHM=:msg=QXBwbHk/\x1b\\";
    for byte in sequence {
        vt.write(&[*byte]);
    }
    let record = &vt.program_status()[0];
    assert_eq!(record.id, "build/test");
    assert_eq!(record.status, AgentStatus::Blocked);
    assert_eq!(record.kind, Some(ProgramStatusKind::Permission));
    assert_eq!(record.progress, Some(42));
    assert_eq!(record.app, "terraform");
    assert_eq!(record.title, "Tests");
    assert_eq!(record.message, "Apply?");
    let revision = record.revision;
    // An identical report is not a new notification, even with a different terminator.
    let mut bel = sequence[..sequence.len() - 2].to_vec();
    bel.push(7);
    vt.write(&bel);
    assert_eq!(vt.program_status()[0].revision, revision);
    report(&mut vt, "state=done:id=build/test");
    let record = &vt.program_status()[0];
    assert_eq!(record.status, AgentStatus::Done);
    assert!(record.app.is_empty() && record.title.is_empty() && record.message.is_empty());
    assert_eq!(record.kind, None);
    assert_eq!(record.progress, None);
    assert!(record.revision > revision);
}

#[test]
fn full_replacement_hierarchy_clear_reset_and_prompt_lifetimes() {
    let mut vt = Terminal::new(80, 24).unwrap();
    for (id, state) in [
        ("", "idle"),
        ("build", "working"),
        ("build/test", "blocked"),
        ("builder", "done"),
        ("failed", "error"),
    ] {
        let address = if id.is_empty() {
            String::new()
        } else {
            format!(":id={id}")
        };
        report(&mut vt, &format!("state={state}{address}"));
    }
    assert_eq!(vt.program_status().len(), 5);
    report(&mut vt, "state=clear:id=build");
    assert_eq!(
        vt.program_status()
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["", "builder", "failed"]
    );
    // Alternate screens and a soft reset do not affect records.
    vt.write(b"\x1b[?1049h\x1b[!p\x1b[?1049l");
    assert_eq!(vt.program_status().len(), 3);
    report(&mut vt, "state=working:id=temporary");
    report(&mut vt, "state=blocked:id=question");
    vt.write(b"\x1b]133;A\x07");
    assert_eq!(
        vt.program_status()
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["builder", "failed"]
    );
    report(&mut vt, "state=working:id=running");
    vt.program_exited();
    assert_eq!(vt.program_status().len(), 2);
    let revision = vt.program_status()[1].revision;
    // RIS triggers a native clear callback. Reused IDs get fresh revisions.
    vt.write(b"\x1bc");
    assert!(vt.program_status().is_empty());
    report(&mut vt, "state=error:id=failed");
    assert!(vt.program_status()[0].revision > revision);
    report(&mut vt, "state=clear");
    assert!(vt.program_status().is_empty());
}

#[test]
fn malformed_unknown_and_oversize_reports_do_not_mutate_records() {
    let mut vt = Terminal::new(80, 24).unwrap();
    report(&mut vt, "state=working:app=valid");
    let before = vt.program_status().to_vec();
    for invalid in [
        "app=missing-state".to_owned(),
        "state=future:app=unknown".into(),
        "state=done:id=/invalid".into(),
        "state=done:id=".into(),
        "state=done:id=bad//id".into(),
        "state=done:id=a/b/c/d/e/f/g/h/i".into(),
        "state=done:msg=A".into(),
        "state=done:msg=AA=A".into(),
        "state=done:msg=Cg==".into(),   // newline
        "state=done:title=CQ==".into(), // tab
        "state=done:msg=fw==".into(),   // DEL
        "state=done:msg=woA=".into(),   // C1 control
        "state=done:msg=/w==".into(),   // invalid UTF-8
        format!("state=done:msg={}", "A".repeat(2733)),
        format!("state=done:title={}", "A".repeat(257)),
        format!("state=done:app={}", "x".repeat(33)),
        format!("state=done:id={}", "x".repeat(33)),
        format!("state=done:{}=x", "k".repeat(17)),
        format!("state=done:future={}", "x".repeat(4096)),
    ] {
        report(&mut vt, &invalid);
        assert_eq!(vt.program_status(), before, "accepted {invalid:?}");
    }
}

#[test]
fn malformed_pairs_are_skipped_unknown_fields_ignored_and_last_key_wins() {
    let mut vt = Terminal::new(80, 24).unwrap();
    report(
        &mut vt,
        " state = idle :broken:state=blocked:kind=question:progress=45:future=ignored:msg=SGk",
    );
    let record = &vt.program_status()[0];
    assert_eq!(record.status, AgentStatus::Blocked);
    assert_eq!(record.kind, Some(ProgramStatusKind::Question));
    assert_eq!(record.progress, Some(45));
    assert_eq!(record.message, "Hi"); // optional base64 padding
    report(&mut vt, "state=done:kind=permission:progress=99");
    assert_eq!(vt.program_status()[0].kind, None);
    assert_eq!(vt.program_status()[0].progress, None);
    report(&mut vt, "state=working:kind=future:progress=101");
    assert_eq!(vt.program_status()[0].kind, None);
    assert_eq!(vt.program_status()[0].progress, None);
}

#[test]
fn records_are_bounded_and_evict_least_recently_updated_including_duplicate_refreshes() {
    let mut vt = Terminal::new(80, 24).unwrap();
    for i in 0..program_status::MAX_RECORDS {
        report(&mut vt, &format!("state=idle:id=r{i}"));
    }
    vt.take_program_status_changed();
    report(&mut vt, "state=idle:id=r0");
    assert!(!vt.take_program_status_changed());
    report(&mut vt, "state=done:id=next");
    assert_eq!(vt.program_status().len(), program_status::MAX_RECORDS);
    assert!(vt.program_status().iter().any(|r| r.id == "r0"));
    assert!(!vt.program_status().iter().any(|r| r.id == "r1"));
    assert!(vt.take_program_status_changed());
}

#[test]
fn plain_output_titles_and_other_notification_protocols_never_create_status() {
    let mut vt = Terminal::new(80, 24).unwrap();
    vt.write(b"Pi is working... Done!\x1b]0;Claude Code\x07\x1b]777;notify;Agent;Done\x07");
    assert!(vt.program_status().is_empty());
    assert!(!vt.take_program_status_changed());
}
