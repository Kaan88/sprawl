//! Bounded newline-delimited stdio for the MCP dispatcher.
use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{dispatch, Call, Event, ToolSpec};
use crate::pipe::SyncIo;

const LINE_LIMIT: usize = 4 << 20;
const INPUT_QUEUE: usize = 8;
const OUTPUT_QUEUE: usize = 16;
const OUTPUT_BYTES: usize = 1 << 20;
const CLOSE_GRACE: Duration = Duration::from_millis(250);
pub(super) const WRITE_LIMIT: Duration = Duration::from_secs(10);
pub(super) const CHECK_OUTPUT: Duration = Duration::from_millis(25);

fn read_input(mut input: impl BufRead, events: mpsc::SyncSender<Event>) {
    let mut line = Vec::new();
    let mut oversized = false;
    while let Ok(buf) = input.fill_buf() {
        if buf.is_empty() {
            if !line.is_empty() { let _ = events.send(Event::Line(line, Instant::now())); }
            break;
        }
        let newline = buf.iter().position(|&b| b == b'\n');
        let n = newline.unwrap_or(buf.len());
        if !oversized {
            if n > LINE_LIMIT - line.len() {
                line.clear();
                oversized = true;
                if events.send(Event::BadLine).is_err() { return; }
            } else {
                line.extend_from_slice(&buf[..n]);
            }
        }
        input.consume(n + usize::from(newline.is_some()));
        if newline.is_some() {
            if !oversized && events.send(Event::Line(std::mem::take(&mut line), Instant::now())).is_err() { return; }
            oversized = false;
        }
    }
    let _ = events.send(Event::Closed);
}

#[derive(Default)]
struct Output {
    aborted: AtomicBool,
    bytes: AtomicUsize,
    writing: Mutex<Option<(Instant, Arc<SyncIo>)>>,
}

impl Output {
    fn abort(&self) {
        self.aborted.store(true, Ordering::Relaxed);
        if let Some((_, io)) = &*self.writing.lock().unwrap_or_else(|e| e.into_inner()) { io.cancel(); }
    }

    fn open(&self) -> bool {
        if self.aborted.load(Ordering::Relaxed) { return false; }
        let writing = self.writing.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((until, io)) = &*writing {
            if Instant::now() >= *until {
                self.aborted.store(true, Ordering::Relaxed);
                io.cancel();
                return false;
            }
        }
        true
    }

    fn push(self: &Arc<Self>, queue: &mpsc::SyncSender<Packet>, value: Value) -> bool {
        if !self.open() { return false; }
        // Reject oversized replies during encoding, not after making an unbounded temporary.
        struct Limited(Vec<u8>);
        impl Write for Limited {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                if buf.len() > OUTPUT_BYTES - 1 - self.0.len() { return Err(io::Error::other("stdio reply too large")); }
                self.0.extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> io::Result<()> { Ok(()) }
        }
        let mut data = Limited(Vec::new());
        if serde_json::to_writer(&mut data, &value).is_err() {
            self.abort();
            return false;
        }
        data.0.push(b'\n');
        let n = data.0.len();
        // One producer; the writer and the channel can only release existing charges.
        if self.bytes.fetch_add(n, Ordering::Relaxed) + n > OUTPUT_BYTES {
            self.bytes.fetch_sub(n, Ordering::Relaxed);
            self.abort();
            return false;
        }
        if queue.try_send(Packet { data: data.0, out: self.clone() }).is_err() {
            self.abort();
            return false;
        }
        true
    }
}

struct Packet { data: Vec<u8>, out: Arc<Output> }

impl Drop for Packet {
    fn drop(&mut self) { self.out.bytes.fetch_sub(self.data.len(), Ordering::Relaxed); }
}

fn write_output(mut sink: impl Write, out: &Output, packets: mpsc::Receiver<Packet>, limit: Duration) {
    let Ok(io) = SyncIo::current().map(Arc::new) else { out.abort(); return; };
    for packet in packets {
        let until = Instant::now() + limit;
        {
            let mut slot = out.writing.lock().unwrap_or_else(|e| e.into_inner());
            if out.aborted.load(Ordering::Relaxed) { return; }
            *slot = Some((until, io.clone()));
        }
        let failed = sink.write_all(&packet.data).and_then(|_| sink.flush()).is_err() || Instant::now() >= until;
        if failed { out.aborted.store(true, Ordering::Relaxed); }
        *out.writing.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if failed { return; }
    }
}

pub(super) fn serve_io(specs: &[ToolSpec], call: Arc<Call>, input: impl BufRead + Send + 'static, sink: impl Write + Send + 'static, limit: Duration) {
    let (events, incoming) = mpsc::sync_channel(INPUT_QUEUE);
    let reader = events.clone();
    std::thread::spawn(move || read_input(input, reader));

    let out = Arc::new(Output::default());
    let writer = out.clone();
    let (send, packets) = mpsc::sync_channel(OUTPUT_QUEUE);
    let (flushed, done) = mpsc::channel();
    std::thread::spawn(move || {
        write_output(sink, &writer, packets, limit);
        let _ = flushed.send(());
    });
    dispatch(specs, call, events, incoming, || out.open(), |r| out.push(&send, r));
    drop(send);
    // Normal EOF/shutdown gets a brief chance to deliver final replies. A blocked Windows
    // writer may not unwind on cancel; the session never joins it (or the stdin reader).
    if out.open() && done.recv_timeout(CLOSE_GRACE).is_err() { out.abort(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs::File;
    use std::io::{BufReader, Cursor};
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows_sys::Win32::System::Pipes::{CreatePipe, PeekNamedPipe};

    fn pipe() -> (File, File) {
        let (mut read, mut write) = (std::ptr::null_mut(), std::ptr::null_mut());
        assert_ne!(unsafe { CreatePipe(&mut read, &mut write, std::ptr::null(), 4096) }, 0);
        unsafe { (File::from_raw_handle(read), File::from_raw_handle(write)) }
    }

    #[test]
    fn oversized_line_is_reported_once_and_the_next_line_is_processed() {
        let mut input = vec![b'x'; LINE_LIMIT + 11];
        input.extend_from_slice(b"\n{\"id\":7,\"method\":\"ping\"}\n");
        let (read, write) = pipe();
        serve_io(&super::super::user_specs(), Arc::new(|_, _, _| unreachable!()), BufReader::new(Cursor::new(input)), write, WRITE_LIMIT);
        let replies: Vec<Value> = BufReader::new(read).lines().map(|line| serde_json::from_str(&line.unwrap()).unwrap()).collect();
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[0]["error"]["code"], -32700);
        assert_eq!(replies[1], json!({"id": 7, "jsonrpc": "2.0", "result": {}}));
    }

    #[derive(Clone, Copy, Debug)]
    enum Ending { Eof, Shutdown, Timeout, Saturation }

    fn full_stdout(ending: Ending) {
        let (read_input, mut send) = pipe();
        let (read_output, write) = pipe();
        let (ready, started) = mpsc::channel();
        let (closed, session_done) = mpsc::channel();
        let timeout = if matches!(ending, Ending::Timeout) { Duration::from_millis(500) } else { Duration::from_secs(5) };
        std::thread::spawn(move || {
            serve_io(&super::super::user_specs(), Arc::new(move |_, _, request| {
                ready.send(request.cancel.clone()).unwrap();
                let until = Instant::now() + Duration::from_secs(5);
                while !request.cancel.stopped() && Instant::now() < until { std::thread::sleep(Duration::from_millis(5)); }
                Ok("fixture".into())
            }), BufReader::new(read_input), write, timeout);
            let _ = closed.send(());
        });
        writeln!(send, "{}", json!({"id": 1, "method": "tools/call", "params": {"name": "search"}})).unwrap();
        let cancel = started.recv_timeout(Duration::from_secs(3)).unwrap();
        writeln!(send, "{}", json!({"id": 2, "method": "tools/list"})).unwrap();
        // Observe real bytes in the OS pipe without draining any. The tools list is larger
        // than this pipe buffer, so the writer cannot finish until the reader consumes them.
        let wait = Instant::now() + Duration::from_secs(2);
        loop {
            let mut pending = 0;
            let peeked = unsafe { PeekNamedPipe(read_output.as_raw_handle(), std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut pending, std::ptr::null_mut()) };
            if peeked != 0 && pending > 0 { break; }
            assert!(Instant::now() < wait, "{ending:?}: no bytes reached stdout");
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut input = Some(send);
        match ending {
            Ending::Eof => drop(input.take()),
            Ending::Shutdown => writeln!(input.as_mut().unwrap(), "{}", json!({"id": 3, "method": "shutdown"})).unwrap(),
            Ending::Timeout => {}
            Ending::Saturation => {
                for id in 3..(OUTPUT_QUEUE + 6) {
                    // The reader may already have dropped its handle after saturation.
                    if writeln!(input.as_mut().unwrap(), "{}", json!({"id": id, "method": "tools/list"})).is_err() { break; }
                }
            }
        }
        session_done.recv_timeout(Duration::from_secs(3)).unwrap_or_else(|_| panic!("{ending:?} didn't end the session"));
        assert!(cancel.stopped(), "{ending:?} did not cancel outstanding work");
        drop(input);
        drop(read_output);
    }

    #[test]
    fn full_windows_stdout_pipe_does_not_prevent_eof_or_shutdown() {
        full_stdout(Ending::Eof);
        full_stdout(Ending::Shutdown);
    }

    #[test]
    fn full_windows_stdout_pipe_aborts_on_write_deadline_or_queue_saturation() {
        full_stdout(Ending::Timeout);
        full_stdout(Ending::Saturation);
    }

    #[test]
    fn queued_bytes_include_the_in_flight_packet() {
        let out = Arc::new(Output::default());
        let (send, recv) = mpsc::sync_channel(OUTPUT_QUEUE);
        let reply = || json!("x".repeat(600 << 10));
        assert!(out.push(&send, reply()));
        let in_flight = recv.recv().unwrap();
        assert!(!out.push(&send, reply()));
        assert!(!out.open());
        drop(in_flight);
        assert_eq!(out.bytes.load(Ordering::Relaxed), 0);
    }
}
