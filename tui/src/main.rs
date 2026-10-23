//! atutop: a btop-style terminal interface for the ATU-100 tuner with the
//! display-buttons-uart firmware. Mouse and keyboard.
//!
//!     atutop [PORT]     PORT: /dev/ttyACM0, COM3, socket://host:port
//!                       default $ATU100_PORT, else /dev/ttyACM0

mod connect;
mod esp;
mod link;
mod tuner;
mod ui;

use esp::{Esp, PROBE};
use link::{Incoming, Link};
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::crossterm::execute;
use std::collections::VecDeque;
use std::io::stdout;
use std::time::{Duration, Instant};
use tuner::{alarm_bcd, Kind, Tuner, CELL_MIN_POWER, CELL_SWR_ALARM, CELL_TUNE_MODE, CELL_WINDING};
use ui::Action;

pub struct App {
    pub port: String,
    pub link_error: Option<String>,
    pub detector: bool, // ask for detector voltages once a second
    pub started: Instant,
    pub esp: Esp,
    pub settings: Option<Settings>, // the settings box, while it is open
    pub connect: Option<connect::Connect>, // the connect screen, while it is open
    sim: Option<connect::Simulator>,       // started from the connect screen
    pub winding: Option<WindingDialog>,    // the detector winding box, while open
    switch_to: Option<String>,             // a port to move to
    /// Whether this computer reaches the bridge's web page: None until checked
    pub web_reach: Option<bool>,
    web_check: Option<std::sync::mpsc::Receiver<bool>>,
}

/// Try the web page's port from here, in the background
fn check_web(url: &str) -> std::sync::mpsc::Receiver<bool> {
    let (tx, rx) = std::sync::mpsc::channel();
    let addr = url.trim_start_matches("http://").trim_end_matches('/').to_string();
    let addr = if addr.contains(':') { addr } else { format!("{addr}:80") };
    std::thread::spawn(move || {
        use std::net::ToSocketAddrs;
        let ok = addr
            .to_socket_addrs()
            .ok()
            .and_then(|mut a| a.next())
            .is_some_and(|a| std::net::TcpStream::connect_timeout(&a, Duration::from_secs(2)).is_ok());
        let _ = tx.send(ok);
    });
    rx
}

pub struct WindingDialog {
    pub choice: u8,    // 10 or 5: the turns ratio 1:10 or 1:5
    pub set_min: bool, // set "Tunes from" to match
}

/// The settings box: the Wi-Fi settings being typed
pub struct Settings {
    pub field: Field,
    pub ssid: String,
    pub pass: String,
    pub show_pass: bool,
    pub selected: Option<usize>, // in the scanned networks
    pub fixed: bool,             // a fixed IP address rather than the router's choice
    pub ip: String,
    pub gateway: String,
    pub mask: String,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Field {
    Network,
    Password,
    Ip,
    Gateway,
    Mask,
}

impl Settings {
    /// The fields showing, in Tab order
    pub fn fields(&self) -> &'static [Field] {
        if self.fixed {
            &[Field::Network, Field::Password, Field::Ip, Field::Gateway, Field::Mask]
        } else {
            &[Field::Network, Field::Password]
        }
    }

    pub fn text(&mut self, f: Field) -> &mut String {
        match f {
            Field::Network => &mut self.ssid,
            Field::Password => &mut self.pass,
            Field::Ip => &mut self.ip,
            Field::Gateway => &mut self.gateway,
            Field::Mask => &mut self.mask,
        }
    }
}

/// Commands wait here and go out one at a time: the tuner reads its serial
/// port between measurements and has a small buffer.
struct Outbox {
    queue: VecDeque<String>,
    sent_at: Option<Instant>,
    recent: VecDeque<(Instant, String)>, // sent in the last second
}

/// What a newer command replaces: {"Set": 55, ...} by cell, others by name.
/// Tune, Reset and the ESP32 bridge's commands always go.
fn replaces(msg: &str) -> Option<&str> {
    let end = if msg.starts_with(r#"{"Set""#) { msg.find(',')? } else { msg.find(':')? };
    let key = &msg[..end];
    (!key.contains("Tune") && !key.contains("Reset") && !key.contains("ESP32")).then_some(key)
}

impl Outbox {
    fn push(&mut self, msg: String) {
        if let Some(key) = replaces(&msg) {
            self.queue.retain(|m| replaces(m) != Some(key));
        }
        self.queue.push_back(msg);
    }

    /// The tuner said something: ready for the next command
    fn answered(&mut self) {
        self.sent_at = None;
    }

    /// A status line the tuner sent before it read our commands would undo
    /// their on-screen change, so a second quick click would start from the
    /// old state. Apply the queued and just-sent commands again on top.
    fn reapply(&mut self, t: &mut Tuner) {
        self.recent.retain(|(at, _)| at.elapsed() < Duration::from_secs(1));
        for msg in self.recent.iter().map(|(_, m)| m).chain(self.queue.iter()) {
            let Ok(serde_json::Value::Object(cmd)) = serde_json::from_str(msg) else { continue };
            let int = |name: &str| cmd.get(name).and_then(|v| v.as_u64());
            if let Some(b) = cmd.get("Auto").and_then(|v| v.as_bool()) {
                t.auto = b;
            }
            if let Some(b) = cmd.get("Bypass").and_then(|v| v.as_bool()) {
                t.bypass = b;
            }
            if let Some(n) = int("RelayI") {
                t.ind_bits = n as u8 & 0x7f;
            }
            if let Some(n) = int("RelayC") {
                t.cap_bits = n as u8 & 0x7f;
                t.order_lc = n & 128 != 0;
            }
            if let (Some(cell), Some(value)) = (int("Set"), int("Value")) {
                if cell < 256 {
                    t.eeprom[cell as usize] = Some(value as u8);
                }
            }
        }
    }

    /// The tuner refused something: let its status stand
    fn refused(&mut self) {
        self.recent.clear();
    }

    /// A new link: nothing sent on it yet
    fn clear(&mut self) {
        self.queue.clear();
        self.recent.clear();
        self.sent_at = None;
    }

    fn idle(&self) -> bool {
        self.queue.is_empty() && self.sent_at.is_none()
    }

    /// Send the next command once the last was answered, or after a second.
    /// While tuning only Reset (to stop it), Status and Raw go: the tuner
    /// answers those while it waits for carrier. The ESP32 bridge answers
    /// its own at any time.
    fn pump(&mut self, link: &mut Link, tuning: bool) {
        if self.sent_at.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            return;
        }
        self.sent_at = None;
        let next_ok = self
            .queue
            .front()
            .is_some_and(|m| !tuning || ["Reset", "Status", "Raw", "ESP32"].iter().any(|k| m.contains(k)));
        if next_ok {
            let msg = self.queue.pop_front().unwrap();
            link.send(&msg);
            self.sent_at = Some(Instant::now());
            self.recent.push_back((Instant::now(), msg));
        }
    }
}

const HELP: &str = "atutop: btop-style control of an ATU-100 tuner

usage: atutop [PORT]

PORT is a serial port (/dev/ttyACM0, COM3) or socket://host:port for a
network bridge; default $ATU100_PORT, else /dev/ttyACM0. 4800 baud.

Without a PORT it opens the connect screen: USB ports, ESP32 bridges found
on the network, and the ports used before (kept in ~/.config/atutop/recent).

keys: t tune, b bypass, a auto, r reset, s status, v detector voltages,
g tuning search, [ ] HighSWR limit, o settings (ESP32 Wi-Fi), c connect
to another port, q or Esc quit. The mouse clicks the buttons, the relays and the tuner settings; the
wheel on HighSWR changes it.";

fn main() {
    let arg = std::env::args().nth(1);
    if matches!(arg.as_deref(), Some("-h" | "--help")) {
        println!("{HELP}");
        return;
    }
    let port = arg.or_else(|| std::env::var("ATU100_PORT").ok());
    let mut app = App { port: String::new(), link_error: None, detector: false, started: Instant::now(), esp: Esp::default(), settings: None, connect: None, sim: None, winding: None, switch_to: None, web_reach: None, web_check: None };
    let mut link = None;
    match port {
        Some(port) => match Link::open(&port) {
            Ok(l) => {
                link = Some(l);
                connect::remember(&port);
                app.port = port;
            }
            Err(e) => {
                eprintln!("atutop: cannot open {e}");
                std::process::exit(1);
            }
        },
        None => app.connect = Some(connect::Connect::new("")),
    }
    let mut t = Tuner::new();
    let mut out = Outbox { queue: VecDeque::new(), sent_at: None, recent: VecDeque::new() };
    if link.is_some() {
        t.log("Connected, asking for the status and settings", Kind::Info);
        out.push(PROBE.into()); // is there an ESP32 bridge?
        app.esp.probing();
        out.push(r#"{"Status": true}"#.into());
    }
    out.push(r#"{"Dump": "EEPROM"}"#.into());

    let mut terminal = ratatui::init();
    let _ = execute!(stdout(), EnableMouseCapture);
    let result = run(&mut terminal, &mut link, &mut out, &mut t, &mut app);
    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    if let Err(e) = result {
        eprintln!("atutop: {e}");
    }
}

fn run(terminal: &mut ratatui::DefaultTerminal, link: &mut Option<Link>, out: &mut Outbox, t: &mut Tuner, app: &mut App) -> std::io::Result<()> {
    let mut last_sample = Instant::now();
    let mut last_poll = Instant::now();
    let mut last_raw = Instant::now();
    let mut last_info = Instant::now();
    let mut last_retry = Instant::now();
    let mut last_web = Instant::now() - Duration::from_secs(60);
    let mut hits = ui::Hits::default();
    loop {
        while let Some(msg) = link.as_ref().and_then(|l| l.rx.try_recv().ok()) {
            match msg {
                Incoming::Line(line) => {
                    if let Ok(serde_json::Value::Object(m)) = serde_json::from_str(&line) {
                        if app.esp.line(&m) {
                            out.answered();
                            continue; // the bridge's own message
                        }
                    }
                    t.line(&line);
                    if line.contains(r#""Event":"Error""#) || line.contains(r#""Event":"Busy""#) {
                        out.refused();
                    }
                    out.reapply(t);
                    out.answered();
                }
                Incoming::Closed(why) => {
                    t.log(format!("Link lost: {why}"), Kind::Bad);
                    app.link_error = Some(why);
                    last_retry = Instant::now();
                }
            }
        }
        // Moving to another port (USB to Wi-Fi), or trying again after the
        // link was lost
        let reopen = app.switch_to.take().or_else(|| {
            (app.link_error.is_some() && last_retry.elapsed() >= Duration::from_secs(2)).then(|| app.port.clone())
        });
        // The simulator: start it, then connect to its ESP32 bridge
        let reopen = match reopen.as_deref() {
            Some(connect::SIM) => {
                app.sim = None; // a fresh one
                match connect::Simulator::start() {
                    Ok(sim) => {
                        let port = sim.port.clone();
                        app.sim = Some(sim);
                        t.log("Simulator started: a pretend tuner and ESP32 bridge", Kind::Info);
                        Some(port)
                    }
                    Err(e) => {
                        if let Some(c) = app.connect.as_mut() {
                            c.error = Some(e);
                        }
                        None
                    }
                }
            }
            _ => reopen,
        };
        if let Some(port) = reopen {
            last_retry = Instant::now();
            let moving = link.is_none() || port != app.port;
            if moving {
                *link = None; // let go of the old port first: it may be the same device
            }
            match Link::open(&port) {
                Ok(new) => {
                    *link = Some(new);
                    t.log(if moving { format!("Connected to {port}") } else { "Link back".into() }, Kind::Good);
                    if moving {
                        connect::remember(&port);
                        app.connect = None;
                        *t = Tuner { events: std::mem::take(&mut t.events), ..Tuner::new() };
                        out.clear();
                        out.push(r#"{"Dump": "EEPROM"}"#.into());
                    }
                    app.port = port;
                    app.link_error = None;
                    app.esp.forget_link();
                    out.queue.retain(|m| m.contains("Dump"));
                    out.queue.push_front(r#"{"Status": true}"#.into());
                    out.queue.push_front(PROBE.into());
                    app.esp.probing();
                }
                Err(e) if moving => {
                    t.log(format!("Cannot connect to {e}"), Kind::Bad);
                    match app.connect.as_mut() {
                        Some(c) => c.error = Some(e),
                        None => app.esp.notice = Some((format!("cannot connect: {e}"), true)),
                    }
                    if !app.port.is_empty() && app.connect.is_none() {
                        app.switch_to = None;
                        app.link_error = Some("not connected".into()); // try the old port again
                    }
                }
                Err(e) => app.link_error = Some(e),
            }
        }
        if let Some(c) = app.connect.as_mut() {
            c.poll();
        }
        app.esp.check_probe();
        // Can this computer reach the bridge's web page?
        if let Some(rx) = &app.web_check {
            if let Ok(ok) = rx.try_recv() {
                app.web_reach = Some(ok);
                app.web_check = None;
            }
        }
        match app.esp.web_url() {
            Some(url) if app.web_check.is_none() && last_web.elapsed() >= Duration::from_secs(10) => {
                last_web = Instant::now();
                app.web_check = Some(check_web(&url));
            }
            None => app.web_reach = None,
            _ => {}
        }
        // While the settings are open, keep the bridge's Wi-Fi state fresh
        let info_every = if app.settings.is_some() { 2 } else { 10 }; // the header shows the web page state
        if app.esp.present == Some(true) && last_info.elapsed() >= Duration::from_secs(info_every) && out.idle() {
            last_info = Instant::now();
            out.push(PROBE.into());
        }
        if last_sample.elapsed() >= Duration::from_millis(500) {
            last_sample = Instant::now();
            t.sample();
        }
        // The tuner sends a status whenever something changes; ask now and
        // then so a silent link shows up (it answers while tuning, too)
        if last_poll.elapsed() >= Duration::from_secs(5) && t.dumped && out.idle() {
            last_poll = Instant::now();
            out.push(r#"{"Status": true}"#.into());
        }
        if app.detector && last_raw.elapsed() >= Duration::from_secs(1) && t.dumped && out.idle() {
            last_raw = Instant::now();
            out.push(r#"{"Raw": true}"#.into());
        }
        if let Some(l) = link.as_mut() {
            out.pump(l, t.tuning);
        }

        terminal.draw(|f| hits = ui::draw(f, t, app))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let action = match event::read()? {
            Event::Key(k) if k.kind == KeyEventKind::Press && app.winding.is_some() => match k.code {
                KeyCode::Esc | KeyCode::Char('q') => Some(Action::WindingCancel),
                KeyCode::Enter => Some(Action::WindingApply),
                KeyCode::Char(' ') => Some(Action::WindingSetMin),
                KeyCode::Up | KeyCode::Char('1') => Some(Action::WindingChoose(10)),
                KeyCode::Down | KeyCode::Char('5') => Some(Action::WindingChoose(5)),
                _ => None,
            },
            Event::Key(k) if k.kind == KeyEventKind::Press && app.connect.is_some() => connect_key(k, app, link.is_some()),
            Event::Key(k) if k.kind == KeyEventKind::Press && app.settings.is_some() => settings_key(k, app),
            Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
                KeyCode::Char('t') => Some(Action::Tune),
                KeyCode::Char('b') => Some(Action::Bypass),
                KeyCode::Char('a') => Some(Action::Auto),
                KeyCode::Char('r') => Some(Action::Reset),
                KeyCode::Char('s') => Some(Action::Status),
                KeyCode::Char('v') => Some(Action::Detector),
                KeyCode::Char('g') => Some(Action::Search),
                KeyCode::Char(']') | KeyCode::Char('+') => Some(Action::AlarmUp),
                KeyCode::Char('[') | KeyCode::Char('-') => Some(Action::AlarmDown),
                KeyCode::Char('o') | KeyCode::F(2) => Some(Action::Settings),
                KeyCode::Char('c') => Some(Action::ChangeLink),
                _ => None,
            },
            Event::Mouse(m) => match m.kind {
                MouseEventKind::Down(MouseButton::Left) => hits.at(m.column, m.row),
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown if app.settings.is_none() && app.connect.is_none() && app.winding.is_none() => {
                    let over = |r: Option<ratatui::layout::Rect>| r.is_some_and(|r| m.row == r.y && m.column >= r.x && m.column < r.x + r.width);
                    let up = m.kind == MouseEventKind::ScrollUp;
                    if over(hits.alarm_row) {
                        Some(if up { Action::AlarmUp } else { Action::AlarmDown })
                    } else {
                        None
                    }
                }
                _ => None,
            },
            _ => None,
        };
        match action {
            Some(Action::Quit) => return Ok(()),
            Some(a) => act(a, out, t, app),
            None => {}
        }
    }
}

/// Keys while the settings box is open: most of them type into its fields
fn settings_key(k: KeyEvent, app: &mut App) -> Option<Action> {
    let s = app.settings.as_mut()?;
    let typing = app.esp.present == Some(true) && app.esp.over_usb;
    match k.code {
        KeyCode::Esc | KeyCode::F(2) => Some(Action::CloseSettings),
        KeyCode::Char('o' | 'q') if !typing => Some(Action::CloseSettings),
        KeyCode::F(5) => Some(Action::Scan),
        _ if !typing => None,
        KeyCode::Tab | KeyCode::BackTab => {
            let fields = s.fields();
            let i = fields.iter().position(|f| *f == s.field).unwrap_or(0);
            let n = fields.len();
            s.field = fields[if k.code == KeyCode::Tab { (i + 1) % n } else { (i + n - 1) % n }];
            None
        }
        KeyCode::Up | KeyCode::Down => {
            let n = app.esp.networks.len();
            if n == 0 {
                return None;
            }
            let i = match (s.selected, k.code) {
                (None, _) => 0,
                (Some(i), KeyCode::Up) => i.saturating_sub(1),
                (Some(i), _) => (i + 1).min(n - 1),
            };
            Some(Action::PickNet(i as u8))
        }
        KeyCode::Enter => Some(Action::Connect),
        KeyCode::Backspace => {
            s.text(s.field).pop();
            None
        }
        KeyCode::Char(c) => {
            let field = s.field;
            let max = match field {
                Field::Network => 32,
                Field::Password => 63,
                _ if !(c.is_ascii_digit() || c == '.') => return None,
                _ => 15,
            };
            let text = s.text(field);
            if text.chars().count() < max {
                text.push(c);
            }
            if s.field == Field::Network {
                s.selected = None;
            }
            None
        }
        _ => None,
    }
}

/// Keys on the connect screen: arrows choose, typing goes to the address
fn connect_key(k: KeyEvent, app: &mut App, linked: bool) -> Option<Action> {
    let c = app.connect.as_mut()?;
    match k.code {
        KeyCode::Esc if linked => Some(Action::CloseConnect),
        KeyCode::Esc => Some(Action::Quit),
        KeyCode::F(5) => Some(Action::ConnectRefresh),
        KeyCode::Enter => Some(Action::ConnectGo),
        KeyCode::Tab | KeyCode::BackTab => {
            c.typing = !c.typing;
            None
        }
        KeyCode::Up | KeyCode::Down => {
            c.typing = false;
            let n = c.entries.len();
            if n > 0 {
                c.selected = if k.code == KeyCode::Up { c.selected.saturating_sub(1) } else { (c.selected + 1).min(n - 1) };
            }
            None
        }
        KeyCode::Backspace => {
            c.address.pop();
            c.typing = true;
            None
        }
        KeyCode::Char(ch) => {
            c.typing = true;
            if c.address.len() < 80 {
                c.address.push(ch);
            }
            None
        }
        _ => None,
    }
}

/// Act on a key or click. Switches and relays change on screen at once;
/// the tuner's status message confirms them.
fn act(action: Action, out: &mut Outbox, t: &mut Tuner, app: &mut App) {
    match action {
        Action::Tune => out.push(r#"{"Tune": true}"#.into()),
        Action::Reset => out.push(r#"{"Reset": true}"#.into()),
        Action::Status => out.push(r#"{"Status": true}"#.into()),
        Action::Bypass => {
            t.bypass = !t.bypass;
            out.push(format!(r#"{{"Bypass": {}}}"#, t.bypass));
        }
        Action::Auto => {
            if t.bypass {
                t.log("Auto mode cannot change while in bypass", Kind::Warn);
            } else {
                t.auto = !t.auto;
                out.push(format!(r#"{{"Auto": {}}}"#, t.auto));
            }
        }
        Action::Detector => {
            app.detector = !app.detector;
            if !app.detector {
                t.fwd_mv = None;
                t.rev_mv = None;
            }
        }
        Action::Ind(i) => {
            t.ind_bits ^= 1 << i;
            out.push(format!(r#"{{"RelayI": {}}}"#, t.ind_bits));
        }
        Action::Cap(i) => {
            t.cap_bits ^= 1 << i;
            out.push(format!(r#"{{"RelayC": {}}}"#, t.cap_bits as u16 | if t.order_lc { 128 } else { 0 }));
        }
        Action::Side => {
            t.order_lc = !t.order_lc;
            out.push(format!(r#"{{"RelayC": {}}}"#, t.cap_bits as u16 | if t.order_lc { 128 } else { 0 }));
        }
        Action::Search => {
            let to_original = t.search_original() != Some(true);
            t.eeprom[CELL_TUNE_MODE] = Some(if to_original { 0 } else { 1 });
            out.push(format!(r#"{{"Set": {CELL_TUNE_MODE}, "Value": {}}}"#, if to_original { 0 } else { 1 }));
            t.log(if to_original { "Tuning search: N7DDC original" } else { "Tuning search: log grid" }, Kind::Info);
        }
        Action::AlarmUp | Action::AlarmDown => {
            // 0 = off, 1.1 to 9.9 in steps of 0.1
            let now = t.alarm().unwrap_or(0.0);
            let next = if action == Action::AlarmUp {
                if now == 0.0 { 1.1 } else { (now + 0.1).min(9.9) }
            } else if now <= 1.1 {
                0.0
            } else {
                now - 0.1
            };
            let value = if next == 0.0 { 0 } else { alarm_bcd(next) };
            t.eeprom[CELL_SWR_ALARM] = Some(value);
            out.push(format!(r#"{{"Set": {CELL_SWR_ALARM}, "Value": {value}}}"#));
        }
        Action::Settings if app.settings.is_some() => app.settings = None,
        Action::Settings => {
            let esp = &mut app.esp;
            app.settings = Some(Settings {
                field: Field::Network,
                ssid: esp.ssid.clone(),
                pass: String::new(),
                show_pass: false,
                selected: None,
                fixed: !esp.static_ip.is_empty(),
                ip: esp.static_ip.clone(),
                gateway: esp.gateway.clone(),
                mask: if esp.mask.is_empty() { "255.255.255.0".into() } else { esp.mask.clone() },
            });
            esp.notice = None;
            if esp.present != Some(true) {
                esp.present = None; // ask again: the bridge may have started since
                esp.probing();
            }
            out.push(PROBE.into());
            if esp.present == Some(true) && esp.over_usb && esp.networks.is_empty() {
                esp.scanning = true;
                out.push(r#"{"ESP32": "Scan"}"#.into());
            }
        }
        Action::CloseSettings => app.settings = None,
        Action::Scan => {
            if app.esp.present == Some(true) && !app.esp.scanning {
                app.esp.scanning = true;
                out.push(r#"{"ESP32": "Scan"}"#.into());
            }
        }
        Action::PickNet(i) => {
            if let (Some(s), Some(net)) = (app.settings.as_mut(), app.esp.networks.get(i as usize)) {
                if s.ssid != net.ssid {
                    s.pass.clear();
                }
                s.ssid = net.ssid.clone();
                s.selected = Some(i as usize);
                s.field = if net.secure { Field::Password } else { Field::Network };
            }
        }
        Action::Focus(field) => {
            if let Some(s) = app.settings.as_mut() {
                s.field = field;
            }
        }
        Action::AddrFixed(fixed) => {
            if let Some(s) = app.settings.as_mut() {
                s.fixed = fixed;
                if fixed && s.ip.is_empty() {
                    // start from the address the router gave it
                    s.ip = app.esp.ip.clone();
                    s.field = Field::Ip;
                } else if !fixed && s.fields().iter().all(|f| *f != s.field) {
                    s.field = Field::Network;
                }
            }
        }
        Action::ShowPass => {
            if let Some(s) = app.settings.as_mut() {
                s.show_pass = !s.show_pass;
            }
        }
        Action::Connect => {
            let Some(s) = app.settings.as_mut() else { return };
            if s.ssid.is_empty() {
                app.esp.notice = Some(("pick a network from the list, or type its name".into(), true));
                return;
            }
            let mut msg = serde_json::json!({"ESP32": "WiFi", "SSID": s.ssid});
            // an empty password for the same network keeps the saved one
            if !s.pass.is_empty() || s.ssid != app.esp.ssid {
                msg["Pass"] = s.pass.clone().into();
            }
            if s.fixed {
                let ip: Option<std::net::Ipv4Addr> = s.ip.parse().ok();
                let mask: Option<std::net::Ipv4Addr> = s.mask.parse().ok();
                let (Some(ip), Some(mask)) = (ip, mask) else {
                    app.esp.notice = Some(("the IP address or mask is not complete, e.g. 192.168.1.60 and 255.255.255.0".into(), true));
                    return;
                };
                let gateway = if s.gateway.is_empty() {
                    let o = ip.octets();
                    format!("{}.{}.{}.1", o[0], o[1], o[2]) // the usual router address
                } else {
                    s.gateway.clone()
                };
                if gateway.parse::<std::net::Ipv4Addr>().is_err() {
                    app.esp.notice = Some(("the gateway address is not complete".into(), true));
                    return;
                }
                s.gateway = gateway.clone();
                msg["IP"] = ip.to_string().into();
                msg["Gateway"] = gateway.into();
                msg["Mask"] = mask.to_string().into();
            } else {
                msg["IP"] = "".into(); // the router's choice
            }
            out.push(msg.to_string());
            app.esp.notice = Some((format!("sending the settings for {}…", s.ssid), false));
            s.pass.clear();
        }
        Action::Forget => out.push(r#"{"ESP32": "Forget"}"#.into()),
        Action::UseWifi => match app.esp.wifi_port() {
            Some(p) => app.switch_to = Some(p),
            None => app.esp.notice = Some(("the bridge is not on Wi-Fi yet".into(), true)),
        },
        Action::ChangeLink => {
            app.settings = None;
            app.connect = Some(connect::Connect::new(&app.port));
        }
        Action::CloseConnect => app.connect = None,
        Action::ConnectRefresh => {
            if let Some(c) = app.connect.as_mut() {
                c.refresh();
            }
        }
        Action::ConnectPick(i) => {
            if let Some(c) = app.connect.as_mut() {
                c.selected = i as usize;
                c.typing = false;
                app.switch_to = c.chosen();
            }
        }
        Action::ConnectType => {
            if let Some(c) = app.connect.as_mut() {
                c.typing = true;
            }
        }
        Action::ConnectGo => {
            if let Some(c) = app.connect.as_mut() {
                c.error = None;
                app.switch_to = c.chosen();
            }
        }
        Action::OpenWeb => {
            if let Some(url) = app.esp.web_url() {
                let opener = if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" };
                let started = std::process::Command::new(opener)
                    .arg(&url)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
                match started {
                    Ok(_) => t.log(format!("Opening {url}"), Kind::Info),
                    Err(e) => t.log(format!("Cannot open a browser: {e}"), Kind::Bad),
                }
            }
        }
        Action::MinToggle => {
            // "Tunes from": 5 W (N7DDC's default) or 1 W (for the 1:5 winding)
            let next = if t.eeprom[CELL_MIN_POWER] == Some(5) { 1 } else { 5 };
            t.eeprom[CELL_MIN_POWER] = Some(next);
            out.push(format!(r#"{{"Set": {CELL_MIN_POWER}, "Value": {next}}}"#));
            if next == 1 && t.winding().is_some_and(|k| k >= 9) && !t.high_power() {
                t.log("Tunes from 1 W: the stock 1:10 winding reads too coarsely below 5 W to tune well", Kind::Warn);
            }
        }
        Action::WindingOpen => {
            let choice = if t.winding() == Some(5) { 5 } else { 10 };
            app.winding = Some(WindingDialog { choice, set_min: true });
        }
        Action::WindingChoose(k) => {
            if let Some(d) = app.winding.as_mut() {
                d.choice = k;
            }
        }
        Action::WindingSetMin => {
            if let Some(d) = app.winding.as_mut() {
                d.set_min = !d.set_min;
            }
        }
        Action::WindingCancel => app.winding = None,
        Action::WindingApply => {
            let Some(d) = app.winding.take() else { return };
            let value = (d.choice / 10) << 4 | d.choice % 10; // BCD: 10 -> 0x10
            t.eeprom[CELL_WINDING] = Some(value);
            out.push(format!(r#"{{"Set": {CELL_WINDING}, "Value": {value}}}"#));
            if d.set_min {
                let min = if d.choice == 5 { 1 } else { 5 };
                t.eeprom[CELL_MIN_POWER] = Some(min);
                out.push(format!(r#"{{"Set": {CELL_MIN_POWER}, "Value": {min}}}"#));
            }
            t.high_power = false;
            t.log(format!("Detector winding set to 1:{}", d.choice), Kind::Warn);
        }
        Action::Quit => {}
    }
}
