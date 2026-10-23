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
    link.send(r#"{"Status": true}"#);
    link.send(r#"{"Dump": "EEPROM"}"#);

    let mut terminal = ratatui::init();
    let _ = execute!(stdout(), EnableMouseCapture);
    let result = run(&mut terminal, &mut link, &mut t, &mut app);
    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    if let Err(e) = result {
        eprintln!("atutop: {e}");
    }
}

fn run(terminal: &mut ratatui::DefaultTerminal, link: &mut Link, t: &mut Tuner, app: &mut App) -> std::io::Result<()> {
    let mut last_sample = Instant::now();
    let mut last_poll = Instant::now();
    let mut last_raw = Instant::now();
    let mut hits = ui::Hits::default();
    loop {
        while let Ok(msg) = link.rx.try_recv() {
            match msg {
                Incoming::Line(line) => t.line(&line),
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
        // then so a silent link shows up, but not while it is busy
        if last_poll.elapsed() >= Duration::from_secs(5) && !t.tuning && t.dumped {
            last_poll = Instant::now();
            link.send(r#"{"Status": true}"#);
        }
        if app.detector && last_raw.elapsed() >= Duration::from_secs(1) && !t.tuning && t.dumped {
            last_raw = Instant::now();
            link.send(r#"{"Raw": true}"#);
        }

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
            Some(a) => act(a, link, t, app),
            None => {}
        }
    }
}

fn act(action: Action, link: &mut Link, t: &mut Tuner, app: &mut App) {
    match action {
        Action::Tune => link.send(r#"{"Tune": true}"#),
        Action::Reset => link.send(r#"{"Reset": true}"#),
        Action::Status => link.send(r#"{"Status": true}"#),
        Action::Bypass => link.send(&format!(r#"{{"Bypass": {}}}"#, !t.bypass)),
        Action::Auto => {
            if t.bypass {
                t.log("Auto mode cannot change while in bypass", Kind::Warn);
            } else {
                link.send(&format!(r#"{{"Auto": {}}}"#, !t.auto));
            }
        }
        Action::Detector => {
            app.detector = !app.detector;
            if !app.detector {
                t.fwd_mv = None;
                t.rev_mv = None;
            }
        }
        Action::Ind(i) => link.send(&format!(r#"{{"RelayI": {}}}"#, t.ind_bits ^ 1 << i)),
        Action::Cap(i) => link.send(&format!(r#"{{"RelayC": {}}}"#, (t.cap_bits ^ 1 << i) as u16 | if t.order_lc { 128 } else { 0 })),
        Action::Side => link.send(&format!(r#"{{"RelayC": {}}}"#, t.cap_bits as u16 | if t.order_lc { 0 } else { 128 })),
        Action::Search => {
            let to_original = t.search_original() != Some(true);
            link.send(&format!(r#"{{"Set": {CELL_TUNE_MODE}, "Value": {}}}"#, if to_original { 0 } else { 1 }));
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
            link.send(&format!(r#"{{"Set": {CELL_SWR_ALARM}, "Value": {value}}}"#));
            // show the new value at once; the tuner's reply confirms it
            t.eeprom[CELL_SWR_ALARM] = Some(value);
        }
        Action::Quit => {}
    }
}
