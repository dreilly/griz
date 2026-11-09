//! Presentation/acknowledgements for protocol records. No agent-specific hooks,
//! process recognition, screen scraping, or heartbeat/PID polling.
use griz_protocol::{AgentInfo, AgentStatus, ProgramStatusRecord};
use std::collections::HashMap;

#[derive(Default)]
pub struct Notifications {
    seen: HashMap<String, u64>,
}

impl Notifications {
    pub fn info(&self, records: &[ProgramStatusRecord], generation: u64) -> Vec<AgentInfo> {
        let mut agents: Vec<_> = records
            .iter()
            .map(|record| {
                let mut resolved = record.clone();
                // Full messages remain in the VT record. The sidebar only needs
                // a preview; bound IPC metadata across every terminal's catalog.
                if resolved.message.len() > 512 {
                    let mut end = 509;
                    while !resolved.message.is_char_boundary(end) {
                        end -= 1;
                    }
                    resolved.message.truncate(end);
                    resolved.message.push('…');
                }
                if resolved.app.is_empty() {
                    let mut id = resolved.id.as_str();
                    while !id.is_empty() {
                        id = id.rsplit_once('/').map_or("", |(parent, _)| parent);
                        if let Some(parent) =
                            records.iter().find(|r| r.id == id && !r.app.is_empty())
                        {
                            resolved.app = parent.app.clone();
                            break;
                        }
                    }
                }
                AgentInfo {
                    unread: matches!(
                        record.status,
                        AgentStatus::Done | AgentStatus::Blocked | AgentStatus::Error
                    ) && self.seen.get(&record.id) != Some(&record.revision),
                    record: resolved,
                    generation,
                }
            })
            .collect();
        agents.sort_by(|a, b| a.record.id.cmp(&b.record.id));
        agents
    }

    pub fn acknowledge(&mut self, records: &[ProgramStatusRecord], id: &str, revision: u64) {
        if records.iter().any(|r| r.id == id && r.revision == revision) {
            self.seen.insert(id.into(), revision);
        }
    }

    /// Bounded by the VT's record cap, including after clear/eviction/reset.
    pub fn reconcile(&mut self, records: &[ProgramStatusRecord]) {
        self.seen
            .retain(|id, _| records.iter().any(|r| &r.id == id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use griz_vt::Terminal;

    #[test]
    fn inheritance_uses_the_nearest_present_ancestor_and_updates_without_child_reports() {
        let mut vt = Terminal::new(80, 24).unwrap();
        vt.write(b"\x1b]7501;state=idle:app=root\x07\x1b]7501;state=working:id=build:app=builder\x07\x1b]7501;state=blocked:id=build/test/check\x07");
        let notifications = Notifications::default();
        let info = notifications.info(vt.program_status(), 1);
        assert_eq!(
            info.iter()
                .find(|a| a.record.id == "build/test/check")
                .unwrap()
                .record
                .app,
            "builder"
        );
        // Replacing build without app removes the old field; its descendants
        // must now inherit root instead. The raw VT record remains app-less.
        vt.write(b"\x1b]7501;state=working:id=build\x07");
        assert_eq!(
            notifications
                .info(vt.program_status(), 1)
                .iter()
                .find(|a| a.record.id == "build/test/check")
                .unwrap()
                .record
                .app,
            "root"
        );
        assert!(
            vt.program_status()
                .iter()
                .find(|r| r.id == "build/test/check")
                .unwrap()
                .app
                .is_empty()
        );
    }

    #[test]
    fn sidebar_previews_are_utf8_safe_and_bounded_without_changing_protocol_records() {
        let mut vt = Terminal::new(80, 24).unwrap();
        let encoded = format!("{}YWE=", "4oKs".repeat(682));
        vt.write(format!("\x1b]7501;state=done:msg={encoded}\x07").as_bytes());
        assert_eq!(vt.program_status()[0].message.len(), 2048);
        let info = Notifications::default().info(vt.program_status(), 1);
        assert!(info[0].record.message.len() <= 512);
        assert!(info[0].record.message.ends_with('…'));
        assert_eq!(vt.program_status()[0].message.len(), 2048);
    }

    #[test]
    fn acknowledgements_cannot_clear_a_newer_record_and_are_pruned_on_clear() {
        let mut vt = Terminal::new(80, 24).unwrap();
        let mut notifications = Notifications::default();
        vt.write(b"\x1b]7501;state=blocked:app=tool\x07");
        let revision = vt.program_status()[0].revision;
        notifications.acknowledge(vt.program_status(), "", revision);
        assert!(!notifications.info(vt.program_status(), 1)[0].unread);
        vt.write(b"\x1b]7501;state=done:app=tool\x07");
        notifications.acknowledge(vt.program_status(), "", revision);
        assert!(notifications.info(vt.program_status(), 1)[0].unread);
        vt.write(b"\x1b]7501;state=clear\x07");
        notifications.reconcile(vt.program_status());
        assert!(notifications.seen.is_empty());
        vt.write(b"\x1b]7501;state=done:app=tool\x07");
        assert!(vt.program_status()[0].revision > revision);
        assert!(notifications.info(vt.program_status(), 1)[0].unread);
    }
}
