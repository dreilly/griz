//! Bounded OSC 7501 record storage. Parsing, validation and capability replies are
//! handled by libghostty-vt; these are the protocol's host-owned lifetime rules.
use griz_protocol::{AgentStatus, ProgramStatusKind, ProgramStatusRecord};
use std::ffi::c_void;

// The spec permits a lower cap, but requires at least 64. Keep catalogs bounded
// across Griz's 64 terminal panes as well as within a single terminal.
pub(crate) const MAX_RECORDS: usize = 64;

#[repr(C)]
pub(crate) struct RawString {
    ptr: *const u8,
    len: usize,
}

/// Frozen GhosttyTerminalProgramStatus ABI from ghostty/vt/terminal.h.
#[repr(C)]
pub(crate) struct RawReport {
    state: u32,
    kind: u32,
    progress: i8,
    id: RawString,
    app: RawString,
    title: RawString,
    message: RawString,
}

#[derive(Default)]
pub(crate) struct Records {
    /// Least recently updated first, including repeated identical reports.
    pub records: Vec<ProgramStatusRecord>,
    revision: u64,
    changed: bool,
}

impl Records {
    fn apply(&mut self, mut report: ProgramStatusRecord) {
        if let Some(index) = self.records.iter().position(|r| r.id == report.id) {
            let old = self.records.remove(index);
            report.revision = old.revision;
            if report == old {
                self.records.push(old);
                return;
            }
        } else if self.records.len() == MAX_RECORDS {
            self.records.remove(0);
        }
        self.revision += 1;
        report.revision = self.revision;
        self.records.push(report);
        self.changed = true;
    }

    fn clear(&mut self, id: &str) {
        let before = self.records.len();
        self.records.retain(|r| {
            !(id.is_empty()
                || r.id == id
                || r.id.strip_prefix(id).is_some_and(|s| s.starts_with('/')))
        });
        self.changed |= before != self.records.len();
    }

    pub fn expire_transient(&mut self) {
        let before = self.records.len();
        // The protocol permits removing idle here too. Completed/failed results
        // deliberately survive both shell prompts and process exit.
        self.records
            .retain(|r| matches!(r.status, AgentStatus::Done | AgentStatus::Error));
        self.changed |= before != self.records.len();
    }

    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }
}

fn string(raw: &RawString) -> String {
    if raw.len == 0 {
        return String::new();
    }
    // SAFETY: libghostty guarantees every string is valid for this callback.
    // Copy immediately: none of these borrowed pointers escape the callback.
    String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(raw.ptr, raw.len) }).into_owned()
}

pub(crate) extern "C" fn report(userdata: *mut c_void, raw: *const RawReport) {
    // SAFETY: Terminal owns this stable Box and the C shim calls synchronously
    // only while it is alive. The raw report is callback-scoped and initialized.
    let (records, raw) = unsafe { (&mut *userdata.cast::<Records>(), &*raw) };
    if raw.state == 5 {
        records.clear(&string(&raw.id));
        return;
    }
    let status = match raw.state {
        0 => AgentStatus::Idle,
        1 => AgentStatus::Working,
        2 => AgentStatus::Done,
        3 => AgentStatus::Blocked,
        4 => AgentStatus::Error,
        _ => return,
    };
    records.apply(ProgramStatusRecord {
        id: string(&raw.id),
        status,
        app: string(&raw.app),
        title: string(&raw.title),
        message: string(&raw.message),
        kind: match raw.kind {
            1 => Some(ProgramStatusKind::Permission),
            2 => Some(ProgramStatusKind::Question),
            3 => Some(ProgramStatusKind::Auth),
            _ => None,
        },
        progress: u8::try_from(raw.progress).ok().filter(|p| *p <= 100),
        revision: 0,
    });
}

pub(crate) extern "C" fn prompt(userdata: *mut c_void) {
    // SAFETY: the same stable, Terminal-owned Box as the report callback.
    unsafe { &mut *userdata.cast::<Records>() }.expire_transient();
}
