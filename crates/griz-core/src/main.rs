//! Durable multi-workspace terminal service. Socket clients own only their view selection.
mod agents;
mod clients;
mod service;
mod session;
mod socket;
mod validation;

use clients::Outbox;
use griz_protocol::{
    Command, Event, Request, TerminalId, WorkspaceAccent, receive, send, socket_path,
};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, mpsc::sync_channel};
use std::time::Duration;

type ClientId = u64;

enum Message {
    Attach {
        client_id: ClientId,
        cols: u16,
        rows: u16,
        default_workspace_accent: WorkspaceAccent,
        outbox: Arc<Outbox>,
    },
    Detach {
        client_id: ClientId,
    },
    Command {
        client_id: ClientId,
        serial: u64,
        command: Command,
    },
    Output {
        terminal_id: TerminalId,
        generation: u64,
        bytes: Vec<u8>,
    },
    Exited {
        terminal_id: TerminalId,
        generation: u64,
        status: String,
    },
    Shutdown,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("griz-core: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut shell = std::env::var_os("SHELL")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| "/bin/sh".into());
    let mut cwd = std::env::current_dir()?;
    let mut stop = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--shell") => shell = args.next().ok_or("--shell needs a path")?.into(),
            Some("--cwd") => cwd = args.next().ok_or("--cwd needs a path")?.into(),
            Some("--stop") => stop = true,
            _ => {
                return Err(
                    "usage: griz-core [--shell PATH] [--cwd PATH] [--stop]; GRIZ_SOCKET overrides endpoint"
                        .into(),
                );
            }
        }
    }

    let path = socket_path()?;
    if stop {
        let mut stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        send(&mut stream, &Request::Shutdown)?;
        if !matches!(receive::<Event>(&mut stream)?, Event::Stopping) {
            return Err("unexpected shutdown response".into());
        }
        return Ok(());
    }

    let endpoint = socket::Endpoint::bind(&path)?;
    let (tx, rx) = sync_channel(1024);
    let mut service = service::Service::new(shell, cwd, tx.clone())?;
    clients::listen(endpoint.listener.try_clone()?, tx);
    eprintln!(
        "griz-core: pid={} socket={}",
        std::process::id(),
        path.display()
    );
    service.run(rx)
}
