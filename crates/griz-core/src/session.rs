//! One PTY process and its bounded I/O workers.
use crate::Message;
use griz_protocol::TerminalId;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{SyncSender, sync_channel},
};

pub struct Session {
    master: Option<Box<dyn MasterPty + Send>>,
    input: Option<SyncSender<Vec<u8>>>,
    killer: Option<Box<dyn ChildKiller + Send + Sync>>,
    exited: Arc<AtomicBool>,
    pub pid: u32,
}

impl Session {
    pub fn spawn(
        shell: &Path,
        cwd: &Path,
        terminal_id: TerminalId,
        generation: u64,
        cols: u16,
        rows: u16,
        tx: SyncSender<Message>,
    ) -> io::Result<Self> {
        let pair = native_pty_system()
            .openpty(size(cols, rows))
            .map_err(io::Error::other)?;
        let mut command = CommandBuilder::new(shell);
        // GUI launches inherit launchd's minimal PATH, not a terminal's shell
        // environment. Login startup (e.g. zsh's .zprofile) must run before the
        // interactive rc file so Homebrew/tools are available to prompt hooks.
        command.args(["-l", "-i"]);
        command.cwd(cwd);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "griz");
        command.env("GRIZ_SESSION", terminal_id.to_string());
        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(io::Error::other)?;
        drop(pair.slave);
        let pid = child
            .process_id()
            .ok_or_else(|| io::Error::other("shell has no PID"))?;
        let killer = child.clone_killer();
        let exited = Arc::new(AtomicBool::new(false));
        let mut reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
        let mut writer = pair.master.take_writer().map_err(io::Error::other)?;
        let (input, rx) = sync_channel::<Vec<u8>>(64);

        let outputs = tx.clone();
        std::thread::spawn(move || {
            let mut buffer = [0; 16 * 1024];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if outputs
                            .send(Message::Output {
                                terminal_id,
                                generation,
                                bytes: buffer[..count].to_vec(),
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    // PTY slave closure is commonly reported as EIO.
                    Err(_) => break,
                }
            }
        });
        std::thread::spawn(move || {
            while let Ok(bytes) = rx.recv() {
                if writer.write_all(&bytes).is_err() {
                    break;
                }
            }
        });
        let wait_exited = exited.clone();
        std::thread::spawn(move || {
            let status = child
                .wait()
                .map(|status| status.to_string())
                .unwrap_or_else(|error| error.to_string());
            wait_exited.store(true, Ordering::Release);
            let _ = tx.send(Message::Exited {
                terminal_id,
                generation,
                status,
            });
        });

        Ok(Self {
            master: Some(pair.master),
            input: Some(input),
            killer: Some(killer),
            exited,
            pid,
        })
    }

    pub fn input(&self) -> Option<&SyncSender<Vec<u8>>> {
        self.input.as_ref()
    }

    /// Query the shell itself so `cd` works without shell-specific prompt hooks.
    pub fn cwd(&self) -> Option<PathBuf> {
        if self.exited.load(Ordering::Acquire) {
            return None;
        }
        process_cwd(self.pid)
    }

    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        self.master
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "PTY is closed"))?
            .resize(size(cols, rows))
            .map_err(io::Error::other)
    }

    /// End only this PTY's foreground process group and shell. The service's own
    /// process group is explicitly excluded before sending a group signal.
    pub fn terminate(&mut self) {
        let already_exited = self.exited.load(Ordering::Acquire);
        if !already_exited
            && let Some(master) = self.master.as_ref()
            && let Some(fd) = master.as_raw_fd()
        {
            // SAFETY: fd is this live PTY master; getpgrp has no preconditions.
            let foreground = unsafe { libc::tcgetpgrp(fd) };
            // SAFETY: getpgrp/getpid have no preconditions.
            let own_group = unsafe { libc::getpgrp() };
            let own_pid = unsafe { libc::getpid() };
            if foreground > 1 && foreground != own_group && foreground != own_pid {
                // SAFETY: a negative pid addresses the foreground group belonging
                // to this PTY. The checks above prevent signaling the service.
                unsafe {
                    libc::kill(-foreground, libc::SIGKILL);
                }
            }
        }
        if let Some(mut killer) = self.killer.take()
            && !already_exited
        {
            let _ = killer.kill();
        }
        self.input.take();
        self.master.take();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(target_os = "macos")]
fn process_cwd(pid: u32) -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::mem::{MaybeUninit, size_of};
    use std::os::unix::ffi::OsStringExt;

    let mut info = MaybeUninit::<libc::proc_vnodepathinfo>::uninit();
    let size = size_of::<libc::proc_vnodepathinfo>();
    // SAFETY: the buffer is writable and exactly the size required for this flavor.
    let written = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            info.as_mut_ptr().cast(),
            size as libc::c_int,
        )
    };
    if written != size as libc::c_int {
        return None;
    }
    // SAFETY: proc_pidinfo reported a complete initialized proc_vnodepathinfo.
    let info = unsafe { info.assume_init() };
    let bytes = info
        .pvi_cdir
        .vip_path
        .iter()
        .flatten()
        .map(|byte| *byte as u8)
        .take_while(|byte| *byte != 0)
        .collect();
    let path = PathBuf::from(OsString::from_vec(bytes));
    path.is_absolute().then_some(path)
}

#[cfg(target_os = "linux")]
fn process_cwd(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn process_cwd(_: u32) -> Option<PathBuf> {
    None
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        cols,
        rows,
        pixel_width: 0,
        pixel_height: 0,
    }
}
