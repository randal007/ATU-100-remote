//! The serial line to the tuner: a serial port, or a TCP bridge given as
//! socket://host:port. A thread reads lines and hands them over a channel.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub enum Incoming {
    Line(String),
    Closed(String),
}

pub struct Link {
    writer: Box<dyn Write + Send>,
    pub rx: Receiver<Incoming>,
    stop: Arc<AtomicBool>, // tells the reading thread to let go of the port
}

impl Drop for Link {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Link {
    pub fn open(port: &str) -> Result<Link, String> {
        let (reader, writer): (Box<dyn Read + Send>, Box<dyn Write + Send>) =
            if let Some(addr) = port.strip_prefix("socket://") {
                let sock = addr
                    .to_socket_addrs()
                    .map_err(|e| format!("{addr}: {e}"))?
                    .next()
                    .ok_or(format!("{addr}: no address"))?;
                let stream = TcpStream::connect_timeout(&sock, Duration::from_secs(3)).map_err(|e| format!("{addr}: {e}"))?;
                stream.set_read_timeout(Some(Duration::from_millis(200))).map_err(|e| e.to_string())?;
                let reader = stream.try_clone().map_err(|e| e.to_string())?;
                (Box::new(reader), Box::new(stream))
            } else {
                let serial = serialport::new(port, 4800)
                    .timeout(Duration::from_millis(200))
                    .open()
                    .map_err(|e| format!("{port}: {e}"))?;
                let reader = serial.try_clone().map_err(|e| e.to_string())?;
                (Box::new(reader), Box::new(serial))
            };

        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 256];
            let mut line = Vec::new();
            while !stopped.load(Ordering::Relaxed) {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        let _ = tx.send(Incoming::Closed("connection closed".into()));
                        return;
                    }
                    Ok(n) => {
                        for &b in &buf[..n] {
                            if b == b'\n' {
                                let text = String::from_utf8_lossy(&line).trim().to_string();
                                line.clear();
                                if !text.is_empty() && tx.send(Incoming::Line(text)).is_err() {
                                    return;
                                }
                            } else if line.len() < 4096 {
                                line.push(b);
                            }
                        }
                    }
                    Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
                    Err(e) => {
                        let _ = tx.send(Incoming::Closed(e.to_string()));
                        return;
                    }
                }
            }
        });
        Ok(Link { writer, rx, stop })
    }

    /// One message to the tuner. The tuner handles them in order.
    pub fn send(&mut self, msg: &str) {
        let _ = self.writer.write_all(msg.as_bytes());
        let _ = self.writer.write_all(b"\n");
        let _ = self.writer.flush();
    }
}
