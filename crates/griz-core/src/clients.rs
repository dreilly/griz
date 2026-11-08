//! Per-client bounded delivery: one replaceable View plus a bounded Error queue.
use crate::{ClientId, Message};
use griz_protocol::{Event, Request, VERSION, receive, send};
use std::collections::VecDeque;
use std::io;
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    mpsc::SyncSender,
};
use std::time::Duration;

const MAX_CLIENTS: usize = 16;
const MAX_ERRORS: usize = 64;

#[derive(Default)]
struct Pending {
    view: Option<Arc<Event>>,
    errors: VecDeque<Arc<Event>>,
    stopping: bool,
    closed: bool,
}

#[derive(Default)]
pub struct Outbox {
    pending: Mutex<Pending>,
    changed: Condvar,
}

impl Outbox {
    pub fn view(&self, event: Event) {
        let mut pending = self.pending.lock().unwrap();
        pending.view = Some(Arc::new(event));
        self.changed.notify_one();
    }

    pub fn error(&self, serial: Option<u64>, message: String) {
        let mut pending = self.pending.lock().unwrap();
        if pending.errors.len() == MAX_ERRORS {
            pending.errors.pop_front();
        }
        pending
            .errors
            .push_back(Arc::new(Event::Error { serial, message }));
        self.changed.notify_one();
    }

    pub fn stopping(&self) {
        let mut pending = self.pending.lock().unwrap();
        pending.stopping = true;
        self.changed.notify_one();
    }

    fn close(&self) {
        let mut pending = self.pending.lock().unwrap();
        pending.closed = true;
        self.changed.notify_all();
    }

    fn next(&self) -> Option<Arc<Event>> {
        let mut pending = self.pending.lock().unwrap();
        loop {
            if let Some(error) = pending.errors.pop_front() {
                return Some(error);
            }
            if pending.stopping {
                pending.stopping = false;
                return Some(Arc::new(Event::Stopping));
            }
            if let Some(view) = pending.view.take() {
                return Some(view);
            }
            if pending.closed {
                return None;
            }
            pending = self.changed.wait(pending).unwrap();
        }
    }
}

pub fn listen(listener: UnixListener, tx: SyncSender<Message>) {
    std::thread::spawn(move || {
        let clients = Arc::new(AtomicUsize::new(0));
        let next_client = Arc::new(AtomicU64::new(1));
        for stream in listener.incoming().flatten() {
            if clients.load(Ordering::Relaxed) >= MAX_CLIENTS {
                continue;
            }
            clients.fetch_add(1, Ordering::Relaxed);
            let client_id = next_client.fetch_add(1, Ordering::Relaxed);
            let (tx, clients) = (tx.clone(), clients.clone());
            std::thread::spawn(move || {
                if let Err(error) = connection(stream, client_id, tx)
                    && !matches!(
                        error.kind(),
                        io::ErrorKind::UnexpectedEof
                            | io::ErrorKind::BrokenPipe
                            | io::ErrorKind::ConnectionReset
                    )
                {
                    eprintln!("griz-core: client: {error}");
                }
                clients.fetch_sub(1, Ordering::Relaxed);
            });
        }
    });
}

fn connection(
    mut stream: UnixStream,
    client_id: ClientId,
    tx: SyncSender<Message>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let request = receive(&mut stream)?;
    let (cols, rows) = match request {
        Request::Shutdown => {
            send(&mut stream, &Event::Stopping)?;
            let _ = tx.send(Message::Shutdown);
            return Ok(());
        }
        Request::Attach {
            version,
            cols,
            rows,
        } if version == VERSION => (cols, rows),
        _ => {
            return send(
                &mut stream,
                &Event::Error {
                    serial: None,
                    message: format!(
                        "expected Attach with protocol version {VERSION}; rebuild/restart the app and core together"
                    ),
                },
            );
        }
    };

    stream.set_read_timeout(None)?;
    let outbox = Arc::new(Outbox::default());
    tx.send(Message::Attach {
        client_id,
        cols,
        rows,
        outbox: outbox.clone(),
    })
    .map_err(io::Error::other)?;

    let mut writer = stream.try_clone()?;
    let alive = Arc::new(AtomicBool::new(true));
    let writer_alive = alive.clone();
    let writer_outbox = outbox.clone();
    let worker = std::thread::spawn(move || {
        while writer_alive.load(Ordering::Acquire) {
            let Some(event) = writer_outbox.next() else {
                break;
            };
            if send(&mut writer, event.as_ref()).is_err() || matches!(*event, Event::Stopping) {
                break;
            }
        }
        writer_alive.store(false, Ordering::Release);
        let _ = writer.shutdown(Shutdown::Both);
    });

    let result = (|| {
        while alive.load(Ordering::Acquire) {
            match receive(&mut stream)? {
                Request::Command { serial, command } => tx
                    .send(Message::Command {
                        client_id,
                        serial,
                        command,
                    })
                    .map_err(io::Error::other)?,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "expected a Command request",
                    ));
                }
            }
        }
        Ok(())
    })();

    let _ = tx.send(Message::Detach { client_id });
    alive.store(false, Ordering::Release);
    outbox.close();
    let _ = stream.shutdown(Shutdown::Both);
    let _ = worker.join();
    result
}
