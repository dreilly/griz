//! The GUI never owns a PTY or emulator. It only attaches to the local service.
use griz_protocol::{
    Command as CoreCommand, Event, Request, ViewState, WorkspaceAccent, receive, send, socket_path,
};
use rune::Waker;
use std::fs::OpenOptions;
use std::io;
use std::net::Shutdown;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{SyncSender, sync_channel},
};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Inbox {
    pub view: Option<ViewState>,
    pub error: Option<String>,
    pub disconnected: bool,
}

pub struct Client {
    outgoing: SyncSender<Request>,
    next_serial: u64,
    pub inbox: Arc<Mutex<Inbox>>,
    stop: Arc<AtomicBool>,
    socket: Arc<Mutex<Option<UnixStream>>>,
}

impl Client {
    #[cfg(test)]
    pub fn for_test() -> (Self, std::sync::mpsc::Receiver<Request>) {
        let (outgoing, receiver) = sync_channel(64);
        (
            Self {
                outgoing,
                next_serial: 1,
                inbox: Arc::new(Mutex::new(Inbox::default())),
                stop: Arc::new(AtomicBool::new(false)),
                socket: Arc::new(Mutex::new(None)),
            },
            receiver,
        )
    }

    pub fn start(grid: (u16, u16), default_workspace_accent: WorkspaceAccent, wake: Waker) -> Self {
        let (outgoing, requests) = sync_channel(64);
        let inbox = Arc::new(Mutex::new(Inbox::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let socket = Arc::new(Mutex::new(None));
        let client = Self {
            outgoing,
            next_serial: 1,
            inbox: inbox.clone(),
            stop: stop.clone(),
            socket: socket.clone(),
        };
        std::thread::spawn(move || {
            let result = (|| -> io::Result<()> {
                let mut stream = connect_or_start(&stop)?;
                stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                send(
                    &mut stream,
                    &Request::Attach {
                        cols: grid.0,
                        rows: grid.1,
                        default_workspace_accent,
                    },
                )?;
                *socket.lock().unwrap() = Some(stream.try_clone()?);
                let mut writer = stream.try_clone()?;
                let writer_inbox = inbox.clone();
                let writer_wake = wake.clone();
                std::thread::spawn(move || {
                    while let Ok(request) = requests.recv() {
                        if let Err(e) = send(&mut writer, &request) {
                            let mut inbox = writer_inbox.lock().unwrap();
                            inbox.error = Some(e.to_string());
                            inbox.disconnected = true;
                            drop(inbox);
                            writer_wake();
                            let _ = writer.shutdown(Shutdown::Both);
                            break;
                        }
                    }
                });
                while !stop.load(Ordering::Acquire) {
                    let event: Event = receive(&mut stream)?;
                    let mut inbox = inbox.lock().unwrap();
                    match event {
                        Event::Error { message, .. } => inbox.error = Some(message),
                        Event::View { state } => inbox.view = Some(state),
                        Event::Stopping => {
                            inbox.error = Some("Core is stopping".into());
                            inbox.disconnected = true;
                        }
                    }
                    drop(inbox);
                    wake();
                }
                Ok(())
            })();
            if let Err(e) = result
                && !stop.load(Ordering::Acquire)
            {
                let mut inbox = inbox.lock().unwrap();
                // Preserve the service's error instead of replacing it with EOF.
                if inbox.error.is_none() {
                    inbox.error = Some(format!("Disconnected: {e}"));
                }
                inbox.disconnected = true;
                drop(inbox);
                wake();
            }
        });
        client
    }

    pub fn send(&mut self, command: CoreCommand) -> Result<u64, String> {
        let serial = self.next_serial;
        self.outgoing
            .try_send(Request::Command { serial, command })
            .map_err(|e| format!("Cannot send to core: {e}"))?;
        self.next_serial += 1;
        Ok(serial)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(stream) = self.socket.lock().unwrap().take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

fn connect_or_start(stop: &AtomicBool) -> io::Result<UnixStream> {
    let path = socket_path()?;
    match UnixStream::connect(&path) {
        Ok(stream) => return Ok(stream),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) => {}
        Err(e) => return Err(e),
    }
    let executable = std::env::current_exe()?;
    let core = executable.with_file_name("griz-core");
    if !core.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "griz-core is missing next to the app; run scripts/dev.sh or cargo build --workspace",
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("invalid socket path"))?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    let metadata = std::fs::symlink_metadata(parent)?;
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "socket directory must be owned by you with mode 0700",
        ));
    }
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(parent.join("core.log"))?;
    let mut command = crate::launch::core_command(&executable)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    // SAFETY: setsid is async-signal-safe and uses no Rust allocation after fork.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    // Reap the spawned process if it exits while the GUI is still alive. Closing the app
    // does not kill it; the OS reparents the detached service.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && !stop.load(Ordering::Acquire) {
        if let Ok(stream) = UnixStream::connect(&path) {
            return Ok(stream);
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "core did not start; see {}",
            parent.join("core.log").display()
        ),
    ))
}
