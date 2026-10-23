//! The connect screen: serial ports by what is plugged in, ESP32 bridges
//! found on the network, the ports used before, and an address to type.

use serde_json::Value;
use std::net::{ToSocketAddrs, UdpSocket};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

pub const FIND_PORT: u16 = 7001; // the bridges answer {"ESP32":"Find"} here
const BRIDGE_PORT: u16 = 7000;
const MDNS_NAME: &str = "atu100.local";
const SIMULATORS: [&str; 2] = ["/tmp/atu100-esp32", "/tmp/atu100-sim"];

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Usb,
    Network,
    Recent,
    Sim,
}

pub const SIM: &str = "sim:";

pub struct Entry {
    pub port: String,
    pub what: String,
    pub kind: Kind,
}

pub struct Connect {
    pub entries: Vec<Entry>,
    pub selected: usize,
    pub typing: bool, // in the address field
    pub address: String,
    pub searching: Option<Instant>,
    pub error: Option<String>,
    found: Option<Receiver<(String, String)>>, // port, description
    pub current: String,
}

/// What a USB serial device probably is, from its USB vendor ID. Only a
/// guess: nothing is opened here (opening a Mega's port restarts it).
fn usb_what(vid: u16, product: Option<&str>) -> String {
    let product = product.map(|p| format!(" · {p}")).unwrap_or_default();
    match vid {
        0x303a => format!("ESP32 (Espressif USB){product}"),
        0x2886 => format!("Seeed XIAO{product}"),
        0x2341 | 0x2a03 => format!("Arduino{product}"),
        0x1a86 => format!("USB serial, CH340 (Arduino clones){product}"),
        0x0403 => format!("USB serial, FTDI{product}"),
        0x10c4 => format!("USB serial, CP210x{product}"),
        _ => format!("USB serial {vid:04x}{product}"),
    }
}

pub fn usb_ports() -> Vec<Entry> {
    let mut list = vec![];
    if let Ok(ports) = serialport::available_ports() {
        for p in ports {
            if let serialport::SerialPortType::UsbPort(u) = &p.port_type {
                list.push(Entry { port: p.port_name.clone(), what: usb_what(u.vid, u.product.as_deref()), kind: Kind::Usb });
            }
        }
    }
    list.sort_by(|a, b| a.port.cmp(&b.port));
    for sim in SIMULATORS {
        if Path::new(sim).exists() {
            let what = if sim.contains("esp32") { "simulated ESP32 bridge" } else { "simulated tuner" };
            list.push(Entry { port: sim.into(), what: what.into(), kind: Kind::Usb });
        }
    }
    list
}

fn recent_file() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(dir.join("atutop").join("recent"))
}

/// Ports used before, the latest first
pub fn recent() -> Vec<String> {
    recent_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|s| s.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect())
        .unwrap_or_default()
}

/// Remember a port that worked
pub fn remember(port: &str) {
    let Some(file) = recent_file() else { return };
    if port.contains("atutop-sim") {
        return; // the simulator's ports are new each time
    }
    let mut list = recent();
    list.retain(|p| p != port);
    list.insert(0, port.to_string());
    list.truncate(8);
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, list.join("\n") + "\n");
}

/// Ask the network for bridges: a broadcast that every bridge answers,
/// and the bridge's mDNS name. Results come back over the channel.
fn search(tx: Sender<(String, String)>) {
    let udp = tx.clone();
    thread::spawn(move || {
        let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else { return };
        let _ = sock.set_broadcast(true);
        let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
        let ask = br#"{"ESP32":"Find"}"#;
        let start = Instant::now();
        let mut buf = [0u8; 512];
        let mut sent = 0;
        while start.elapsed() < Duration::from_secs(2) {
            if sent < 3 && start.elapsed() >= Duration::from_millis(500 * sent) {
                let _ = sock.send_to(ask, ("255.255.255.255", FIND_PORT));
                let _ = sock.send_to(ask, ("127.0.0.1", FIND_PORT)); // a simulator on this computer
                sent += 1;
            }
            if let Ok((n, from)) = sock.recv_from(&mut buf) {
                let Ok(Value::Object(m)) = serde_json::from_slice(&buf[..n]) else { continue };
                if m.get("ESP32").and_then(|v| v.as_str()) != Some("Here") {
                    continue;
                }
                let ip = m.get("IP").and_then(|v| v.as_str()).map(String::from).unwrap_or(from.ip().to_string());
                let port = m.get("Port").and_then(|v| v.as_u64()).unwrap_or(BRIDGE_PORT as u64);
                let mut what = "ESP32 bridge".to_string();
                if let Some(h) = m.get("Host").and_then(|v| v.as_str()).filter(|h| !h.is_empty()) {
                    what += &format!(" · {h}");
                }
                if let Some(s) = m.get("SSID").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                    what += &format!(" · on {s}");
                }
                let _ = udp.send((format!("socket://{ip}:{port}"), what));
            }
        }
    });
    thread::spawn(move || {
        if let Ok(mut addrs) = (MDNS_NAME, BRIDGE_PORT).to_socket_addrs() {
            if let Some(a) = addrs.find(|a| a.is_ipv4()) {
                let _ = tx.send((format!("socket://{}:{}", a.ip(), a.port()), format!("ESP32 bridge · {MDNS_NAME}")));
            }
        }
    });
}

/// What was typed, as a port: an IP address or name gets the bridge's port
pub fn port_from(text: &str) -> String {
    let t = text.trim();
    if t.starts_with("socket://") || t.starts_with('/') || t.to_uppercase().starts_with("COM") {
        t.to_string()
    } else if t.contains(':') {
        format!("socket://{t}")
    } else {
        format!("socket://{t}:{BRIDGE_PORT}")
    }
}

impl Connect {
    /// `current`: the port in use now, "" for none
    pub fn new(current: &str) -> Connect {
        let mut c = Connect { entries: vec![], selected: 0, typing: false, address: String::new(), searching: None, error: None, found: None, current: current.to_string() };
        c.refresh();
        c
    }

    /// Look again: USB ports now, the network over the next two seconds
    pub fn refresh(&mut self) {
        let keep = self.entries.get(self.selected).map(|e| e.port.clone());
        self.entries = usb_ports();
        if !self.current.is_empty() && !self.entries.iter().any(|e| e.port == self.current) {
            let kind = if self.current.starts_with("socket://") { Kind::Network } else { Kind::Usb };
            let what = if self.current.contains("atutop-sim") { "the simulator" } else { "" };
            self.entries.push(Entry { port: self.current.clone(), what: what.into(), kind });
        }
        for port in recent() {
            if !self.entries.iter().any(|e| e.port == port) && !(port.starts_with('/') && !Path::new(&port).exists()) {
                self.entries.push(Entry { port, what: "used before".into(), kind: Kind::Recent });
            }
        }
        if sim_dir().is_some() {
            self.entries.push(Entry {
                port: SIM.into(),
                what: "pretend tuner and ESP32 bridge, no hardware".into(),
                kind: Kind::Sim,
            });
        }
        // start on the last one used, when it is here
        let keep = keep.or_else(|| (!self.current.is_empty()).then(|| self.current.clone())).or_else(|| recent().into_iter().next());
        self.order(keep);
        let (tx, rx) = mpsc::channel();
        search(tx);
        self.found = Some(rx);
        self.searching = Some(Instant::now());
    }

    /// Take in bridges found on the network
    pub fn poll(&mut self) {
        let found: Vec<(String, String)> = self.found.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        {
            for (port, what) in found {
                match self.entries.iter_mut().find(|e| e.port == port) {
                    Some(e) => {
                        e.what = what;
                        e.kind = Kind::Network;
                    }
                    None => self.entries.push(Entry { port, what, kind: Kind::Network }),
                }
                let keep = self.entries.get(self.selected).map(|e| e.port.clone());
                self.order(keep);
            }
        }
        if self.searching.is_some_and(|t| t.elapsed() > Duration::from_secs(3)) {
            self.searching = None;
        }
    }

    /// USB, then the network, then the rest, keeping the highlight on `keep`
    fn order(&mut self, keep: Option<String>) {
        let rank = |k: Kind| match k {
            Kind::Usb => 0,
            Kind::Network => 1,
            Kind::Recent => 2,
            Kind::Sim => 3,
        };
        self.entries.sort_by_key(|e| rank(e.kind));
        self.selected = keep.and_then(|k| self.entries.iter().position(|e| e.port == k)).unwrap_or(0);
    }

    /// The port to open: the typed address, or the highlighted entry
    pub fn chosen(&self) -> Option<String> {
        if self.typing && !self.address.trim().is_empty() {
            Some(port_from(&self.address))
        } else {
            self.entries.get(self.selected).map(|e| e.port.clone())
        }
    }
}

/// Where the simulator lives: $ATU100_SIM_DIR, else the test folder of the
/// source tree atutop was built from
fn sim_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("ATU100_SIM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../ATU_100_EXT_board/FirmWare_PIC16F1938/test"));
    (dir.join("host_test").exists() && dir.join("esp32_sim.py").exists()).then_some(dir)
}

/// The simulated tuner (the firmware's own code, built for the PC) with the
/// simulated ESP32 bridge in front of it. Both stop when this is dropped.
pub struct Simulator {
    children: Vec<Child>,
    link: PathBuf,
    pub port: String,
}

impl Drop for Simulator {
    fn drop(&mut self) {
        for c in &mut self.children {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_file(&self.link);
    }
}

/// Start a program and wait for the line that names its serial port; the
/// rest of its output is read and thrown away so it never blocks.
fn spawn_until(cmd: &mut Command, prefix: &'static str) -> Result<(Child, String), String> {
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    let out = child.stdout.take().ok_or("no output")?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(rest) = line.strip_prefix(prefix) {
                let _ = tx.send(rest.split_whitespace().next().unwrap_or("").to_string());
            }
        }
    });
    match rx.recv_timeout(Duration::from_secs(4)) {
        Ok(port) if !port.is_empty() => Ok((child, port)),
        _ => {
            let _ = child.kill();
            Err(format!("{} did not start", cmd.get_program().to_string_lossy()))
        }
    }
}

impl Simulator {
    pub fn start() -> Result<Simulator, String> {
        let dir = sim_dir().ok_or("the simulator is not built: run test/run.sh")?;
        let (tuner, tuner_port) = spawn_until(
            Command::new(dir.join("host_test")).current_dir(&dir).env("PTY", "1").env("QUIET", "1").env("RF_WAVE", "1"),
            "serial port: ",
        )?;
        let mut sim = Simulator { children: vec![tuner], link: std::env::temp_dir().join(format!("atutop-sim-{}", std::process::id())), port: String::new() };
        let (esp, _) = spawn_until(
            Command::new("python3")
                .arg(dir.join("esp32_sim.py"))
                .arg(&tuner_port)
                .arg(&sim.link)
                .arg("--watch")
                .arg(sim.children[0].id().to_string())
                .current_dir(&dir),
            "USB port: ",
        )
        .map_err(|_| "the ESP32 simulator did not start (is another one using port 7000?)".to_string())?;
        sim.children.push(esp);
        sim.port = sim.link.to_string_lossy().into_owned();
        Ok(sim)
    }
}
