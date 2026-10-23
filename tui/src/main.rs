//! atutop: a btop-style terminal interface for the ATU-100 tuner with the
//! display-buttons-uart firmware. Mouse and keyboard.
//!
//!     atutop [PORT]     PORT: /dev/ttyACM0, COM3, socket://host:port
//!                       default $ATU100_PORT, else /dev/ttyACM0

mod link;
mod tuner;
mod ui;

use link::{Incoming, Link};
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use ratatui::crossterm::execute;
use std::collections::VecDeque;
use std::io::stdout;
use std::time::{Duration, Instant};
use tuner::{alarm_bcd, Kind, Tuner, CELL_SWR_ALARM, CELL_TUNE_MODE};
use ui::Action;

pub struct App {
    pub port: String,
    pub link_error: Option<String>,
    pub detector: bool, // ask for detector voltages once a second
    pub started: Instant,
}

/// Commands wait here and go out one at a time: the tuner reads its serial
/// port between measurements and has a small buffer.
struct Outbox {
    queue: VecDeque<String>,
    sent_at: Option<Instant>,
    recent: VecDeque<(Instant, String)>, // sent in the last second
}

/// What a newer command replaces: {"Set": 55, ...} by cell, others by name.
/// Tune and Reset always go.
fn replaces(msg: &str) -> Option<&str> {
    let end = if msg.starts_with(r#"{"Set""#) { msg.find(',')? } else { msg.find(':')? };
    let key = &msg[..end];
    (!key.contains("Tune") && !key.contains("Reset")).then_some(key)
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

    fn idle(&self) -> bool {
        self.queue.is_empty() && self.sent_at.is_none()
    }

    /// Send the next command once the last was answered, or after a second.
    /// While tuning only Reset (to stop it), Status and Raw go: the tuner
    /// answers those while it waits for carrier.
    fn pump(&mut self, link: &mut Link, tuning: bool) {
        if self.sent_at.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            return;
        }
        self.sent_at = None;
        let next_ok = self
            .queue
            .front()
            .is_some_and(|m| !tuning || ["Reset", "Status", "Raw"].iter().any(|k| m.contains(k)));
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

keys: t tune, b bypass, a auto, r reset, s status, v detector voltages,
g tuning search, [ ] HighSWR limit, q or Esc quit. The mouse clicks the
buttons, the relays and the tuner settings; the wheel on HighSWR changes it.";

fn main() {
    let arg = std::env::args().nth(1);
    if matches!(arg.as_deref(), Some("-h" | "--help")) {
        println!("{HELP}");
        return;
    }
    let port = arg.or_else(|| std::env::var("ATU100_PORT").ok()).unwrap_or_else(|| "/dev/ttyACM0".into());
    let mut link = match Link::open(&port) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("atutop: cannot open {e}");
            std::process::exit(1);
        }
    };
    let mut app = App { port, link_error: None, detector: false, started: Instant::now() };
    let mut t = Tuner::new();
    t.log("Connected, asking for the status and settings", Kind::Info);
    let mut out = Outbox { queue: VecDeque::new(), sent_at: None, recent: VecDeque::new() };
    out.push(r#"{"Status": true}"#.into());
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

fn run(terminal: &mut ratatui::DefaultTerminal, link: &mut Link, out: &mut Outbox, t: &mut Tuner, app: &mut App) -> std::io::Result<()> {
    let mut last_sample = Instant::now();
    let mut last_poll = Instant::now();
    let mut last_raw = Instant::now();
    let mut hits = ui::Hits::default();
    loop {
        while let Ok(msg) = link.rx.try_recv() {
            match msg {
                Incoming::Line(line) => {
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
                }
            }
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
        out.pump(link, t.tuning);

        terminal.draw(|f| hits = ui::draw(f, t, app))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let action = match event::read()? {
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
                _ => None,
            },
            Event::Mouse(m) => match m.kind {
                MouseEventKind::Down(MouseButton::Left) => hits.at(m.column, m.row),
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                    let over_alarm = hits.alarm_row.is_some_and(|r| m.row == r.y && m.column >= r.x && m.column < r.x + r.width);
                    match (over_alarm, m.kind) {
                        (true, MouseEventKind::ScrollUp) => Some(Action::AlarmUp),
                        (true, _) => Some(Action::AlarmDown),
                        _ => None,
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
        Action::Quit => {}
    }
}
