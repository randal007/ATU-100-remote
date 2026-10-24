//! atutop's state and behaviour, shared by the terminal program and the
//! browser version: what the tuner and the bridge said, the commands going
//! out, the boxes that are open, and what keys and clicks do. The two
//! front ends only carry lines in and out, turn their keys and clicks into
//! `Key` and `Mouse`, and do the few things only they can (`Effect`).

use crate::cells::{self, Kind as CellKind, CELLS};
use crate::connect;
use crate::esp::{Esp, PROBE};
use crate::tuner::{alarm_bcd, Kind, Tuner, CELL_MIN_POWER, CELL_SWR_ALARM, CELL_TUNE_MODE, CELL_WINDING};
use crate::ui::{self, Action, Hits};
use ratatui::Frame;
use std::collections::VecDeque;
use std::time::Duration;
use web_time::Instant;

pub struct App {
    pub port: String,
    pub link_error: Option<String>,
    pub detector: bool, // ask for detector voltages once a second
    pub started: Instant,
    pub esp: Esp,
    pub settings: Option<Settings>,        // the settings box, while it is open
    pub connect: Option<connect::Connect>, // the connect screen, while it is open
    pub winding: Option<WindingDialog>,    // the detector winding box, while open
    pub atu: Option<AtuBox>,               // the ATU-100 settings box, while open
    /// Which of the bridge's web addresses this computer reaches: None
    /// until checked, Some(None) when none
    pub web_reach: Option<Option<String>>,
    /// Running in a web browser, served by the bridge itself
    pub in_browser: bool,
}

/// The ATU-100 settings box: the tuner's EEPROM cells
pub struct AtuBox {
    pub selected: usize, // in cells::CELLS
    pub edit: String,    // the value being typed for the selected cell
    pub confirm_reset: bool,
    pub notice: Option<(String, bool)>, // text, is it an error
    /// First row of the list on screen, when scrolled by hand (the bar, the
    /// wheel); None follows the picked cell
    pub scroll: Option<usize>,
    /// First row drawn last time, for the wheel to scroll from
    pub top_drawn: std::cell::Cell<usize>,
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
    AtuValue, // the ATU-100 settings box's value
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
            Field::AtuValue => &mut self.mask, // not a network setting: never asked for
        }
    }

    /// Longest text a field takes, and whether a character belongs in it
    pub fn accepts(f: Field, c: char) -> (usize, bool) {
        match f {
            Field::Network => (32, true),
            Field::Password => (63, true),
            Field::AtuValue => (6, c.is_ascii_digit() || c == '.'),
            _ => (15, c.is_ascii_digit() || c == '.'),
        }
    }
}

/// Keys, the same from the terminal and the browser
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Key {
    Char(char),
    Enter,
    Esc,
    Tab,
    BackTab,
    Backspace,
    Up,
    Down,
    F(u8),
}

/// Mouse buttons and the wheel, at a screen cell
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mouse {
    Click(u16, u16),
    /// Moved with the button held (or a finger down)
    Drag(u16, u16),
    Release,
    WheelUp(u16, u16),
    WheelDown(u16, u16),
}

/// What only the front end can do
#[derive(Clone, PartialEq, Debug)]
pub enum Effect {
    Quit,
    OpenWeb(String),
    SwitchTo(String), // connect to another port
    OpenConnect,      // the connect screen (terminal only)
    ConnectRefresh,
    ConnectPick(u8),
    ConnectGo,
}

/// Commands wait here and go out one at a time: the tuner reads its serial
/// port between measurements and has a small buffer.
pub struct Outbox {
    pub queue: VecDeque<String>,
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
    pub fn new() -> Outbox {
        Outbox { queue: VecDeque::new(), sent_at: None, recent: VecDeque::new() }
    }

    pub fn push(&mut self, msg: String) {
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
    pub fn clear(&mut self) {
        self.queue.clear();
        self.recent.clear();
        self.sent_at = None;
    }

    pub fn idle(&self) -> bool {
        self.queue.is_empty() && self.sent_at.is_none()
    }

    /// Send the next command once the last was answered, or after a second.
    /// While tuning only Reset (to stop it), Status and Raw go: the tuner
    /// answers those while it waits for carrier. The ESP32 bridge answers
    /// its own at any time.
    fn pump(&mut self, send: &mut dyn FnMut(&str), tuning: bool) {
        if self.sent_at.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            return;
        }
        self.sent_at = None;
        // While tuning, the first command allowed then, wherever it is in
        // the queue: a Bypass waiting at the front must not hold back a Reset
        let allowed = |m: &String| !tuning || ["Reset", "Status", "Raw", "ESP32"].iter().any(|k| m.contains(k));
        if let Some(i) = self.queue.iter().position(allowed) {
            let msg = self.queue.remove(i).unwrap();
            send(&msg);
            self.sent_at = Some(Instant::now());
            self.recent.push_back((Instant::now(), msg));
        }
    }
}

/// Everything atutop knows and does, apart from the link and the screen
pub struct Core {
    dragging: bool, // a scroll bar is being dragged
    pub t: Tuner,
    pub out: Outbox,
    pub app: App,
    pub hits: Hits,
    last_sample: Instant,
    last_poll: Instant,
    last_raw: Instant,
    last_info: Instant,
}

impl Core {
    pub fn new(in_browser: bool) -> Core {
        let now = Instant::now();
        Core {
            dragging: false,
            t: Tuner::new(),
            out: Outbox::new(),
            app: App {
                port: String::new(),
                link_error: None,
                detector: false,
                started: now,
                esp: Esp::default(),
                settings: None,
                connect: None,
                winding: None,
                atu: None,
                web_reach: None,
                in_browser,
            },
            hits: Hits::default(),
            last_sample: now,
            last_poll: now,
            last_raw: now,
            last_info: now,
        }
    }

    /// A link is open. `fresh`: a new port, so forget the old tuner;
    /// otherwise the same one came back after a dropout.
    pub fn connected(&mut self, port: &str, fresh: bool) {
        if fresh {
            self.t = Tuner { events: std::mem::take(&mut self.t.events), ..Tuner::new() };
            self.out.clear();
            self.out.push(r#"{"Dump": "EEPROM"}"#.into());
        }
        self.app.port = port.to_string();
        self.app.link_error = None;
        // a Tuned may have been lost with the link: the status will tell
        self.t.tuning = false;
        self.app.esp.forget_link();
        self.out.queue.retain(|m| m.contains("Dump"));
        self.out.queue.push_front(r#"{"Status": true}"#.into());
        self.out.queue.push_front(PROBE.into()); // is there an ESP32 bridge?
        self.app.esp.probing();
    }

    pub fn lost(&mut self, why: String) {
        self.t.log(format!("Link lost: {why}"), Kind::Bad);
        self.app.link_error = Some(why);
    }

    /// A whole line from the link
    pub fn line(&mut self, line: &str) {
        if let Ok(serde_json::Value::Object(m)) = serde_json::from_str(line) {
            if self.app.esp.line(&m) {
                self.out.answered();
                return; // the bridge's own message
            }
        }
        self.t.line(line);
        if line.contains(r#""Event":"Error""#) || line.contains(r#""Event":"Busy""#) {
            self.out.refused();
        }
        self.out.reapply(&mut self.t);
        self.out.answered();
    }

    /// Now and then: graphs, polls, and the next command onto the link
    pub fn tick(&mut self, send: &mut dyn FnMut(&str)) {
        self.app.esp.check_probe();
        // While the settings are open, keep the bridge's Wi-Fi state fresh
        let info_every = if self.app.settings.is_some() { 2 } else { 10 }; // the header shows the web page state
        if self.app.esp.present == Some(true) && self.last_info.elapsed() >= Duration::from_secs(info_every) && self.out.idle() {
            self.last_info = Instant::now();
            self.out.push(PROBE.into());
        }
        if self.last_sample.elapsed() >= Duration::from_millis(500) {
            self.last_sample = Instant::now();
            self.t.sample();
        }
        // The tuner sends a status whenever something changes; ask now and
        // then so a silent link shows up (it answers while tuning, too)
        if self.last_poll.elapsed() >= Duration::from_secs(5) && self.t.dumped && self.out.idle() {
            self.last_poll = Instant::now();
            self.out.push(r#"{"Status": true}"#.into());
        }
        if self.app.detector && self.last_raw.elapsed() >= Duration::from_secs(1) && self.t.dumped && self.out.idle() {
            self.last_raw = Instant::now();
            self.out.push(r#"{"Raw": true}"#.into());
        }
        if self.app.link_error.is_none() && !self.app.port.is_empty() {
            self.out.pump(send, self.t.tuning);
        }
    }

    pub fn draw(&mut self, f: &mut Frame) {
        self.hits = ui::draw(f, &self.t, &self.app);
    }

    pub fn mouse(&mut self, m: Mouse) -> Option<Effect> {
        // the wheel scrolls the ATU-100 settings list
        if let (Some(a), Mouse::WheelUp(..) | Mouse::WheelDown(..)) = (self.app.atu.as_mut(), m) {
            if self.app.winding.is_none() {
                let top = a.top_drawn.get();
                a.scroll = Some(if matches!(m, Mouse::WheelUp(..)) { top.saturating_sub(3) } else { top + 3 });
                return None;
            }
        }
        // Dragging a scroll bar: once it started on the bar, any row the
        // pointer is on counts, however far it strays sideways
        match m {
            Mouse::Release => {
                self.dragging = false;
                return None;
            }
            Mouse::Drag(_, y) => {
                if !self.dragging {
                    return None;
                }
                let a = self.hits.areas.iter().find(|(r, a)| r.y == y && matches!(a, Action::AtuScroll(_))).map(|(_, a)| *a);
                return a.and_then(|a| self.act(a));
            }
            Mouse::Click(x, y) => self.dragging = matches!(self.hits.at(x, y), Some(Action::AtuScroll(_))),
            _ => {}
        }
        let modal = self.app.settings.is_some() || self.app.connect.is_some() || self.app.winding.is_some() || self.app.atu.is_some();
        let over = |r: Option<ratatui::layout::Rect>, x: u16, y: u16| r.is_some_and(|r| y == r.y && x >= r.x && x < r.x + r.width);
        let action = match m {
            Mouse::Click(x, y) => self.hits.at(x, y),
            Mouse::WheelUp(x, y) if !modal && over(self.hits.alarm_row, x, y) => Some(Action::AlarmUp),
            Mouse::WheelDown(x, y) if !modal && over(self.hits.alarm_row, x, y) => Some(Action::AlarmDown),
            _ => None,
        };
        action.and_then(|a| self.act(a))
    }

    pub fn key(&mut self, k: Key) -> Option<Effect> {
        let action = if self.app.winding.is_some() {
            match k {
                Key::Esc | Key::Char('q') => Some(Action::WindingCancel),
                Key::Enter => Some(Action::WindingApply),
                Key::Char(' ') => Some(Action::WindingSetMin),
                Key::Up | Key::Char('1') => Some(Action::WindingChoose(10)),
                Key::Down | Key::Char('5') => Some(Action::WindingChoose(5)),
                _ => None,
            }
        } else if self.app.connect.is_some() {
            self.connect_key(k)
        } else if self.app.settings.is_some() {
            self.settings_key(k)
        } else if self.app.atu.is_some() {
            self.atu_key(k)
        } else {
            match k {
                Key::Char('q') | Key::Esc if !self.app.in_browser => Some(Action::Quit),
                Key::Char('t') => Some(Action::Tune),
                Key::Char('b') => Some(Action::Bypass),
                Key::Char('a') => Some(Action::Auto),
                Key::Char('r') => Some(Action::Reset),
                Key::Char('s') => Some(Action::Status),
                Key::Char('v') => Some(Action::Detector),
                Key::Char('g') => Some(Action::Search),
                Key::Char(']') | Key::Char('+') => Some(Action::AlarmUp),
                Key::Char('[') | Key::Char('-') => Some(Action::AlarmDown),
                Key::Char('o') | Key::F(2) => Some(Action::Settings),
                Key::Char('e') => Some(Action::AtuOpen),
                Key::Char('c') if !self.app.in_browser => Some(Action::ChangeLink),
                _ => None,
            }
        };
        action.and_then(|a| self.act(a))
    }

    /// May the Wi-Fi settings be changed from here? Over USB, or from the
    /// bridge's own hotspot
    pub fn may_set_wifi(&self) -> bool {
        self.app.esp.present == Some(true) && self.app.esp.may_set_wifi
    }

    /// The text boxes showing now: which field, its text, and whether it
    /// is a password (for the browser's real text boxes)
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn text_boxes(&self) -> Vec<(Field, String, bool)> {
        let mut list = vec![];
        if let Some(s) = self.app.settings.as_ref().filter(|_| self.may_set_wifi()) {
            for f in s.fields() {
                let text = match f {
                    Field::Network => &s.ssid,
                    Field::Password => &s.pass,
                    Field::Ip => &s.ip,
                    Field::Gateway => &s.gateway,
                    _ => &s.mask,
                };
                list.push((*f, text.clone(), *f == Field::Password && !s.show_pass));
            }
        }
        if let Some(a) = self.app.atu.as_ref().filter(|a| cells::typed(&CELLS[a.selected])) {
            list.push((Field::AtuValue, a.edit.clone(), false));
        }
        list
    }

    /// The field atutop has selected, if a box with fields is open
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn selected_field(&self) -> Option<Field> {
        if self.app.atu.is_some() {
            return Some(Field::AtuValue);
        }
        self.app.settings.as_ref().map(|s| s.field)
    }

    /// Text typed into the focused settings field (the browser's phone
    /// keyboard sends whole texts)
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn set_field_text(&mut self, text: &str) {
        if let Some(a) = self.app.atu.as_mut() {
            let (max, _) = Settings::accepts(Field::AtuValue, ' ');
            a.edit = text.chars().filter(|c| Settings::accepts(Field::AtuValue, *c).1).take(max).collect();
            return;
        }
        let Some(s) = self.app.settings.as_mut() else { return };
        let field = s.field;
        let (max, _) = Settings::accepts(field, ' ');
        let clean: String = text.chars().filter(|c| Settings::accepts(field, *c).1).take(max).collect();
        *s.text(field) = clean;
        if field == Field::Network {
            s.selected = None;
        }
    }

    /// Keys while the settings box is open: most of them type into its fields
    fn settings_key(&mut self, k: Key) -> Option<Action> {
        let typing = self.may_set_wifi();
        let n_nets = self.app.esp.networks.len();
        let s = self.app.settings.as_mut()?;
        match k {
            Key::Esc | Key::F(2) => Some(Action::CloseSettings),
            Key::Char('o' | 'q') if !typing => Some(Action::CloseSettings),
            Key::F(5) => Some(Action::Scan),
            _ if !typing => None,
            Key::Tab | Key::BackTab => {
                let fields = s.fields();
                let i = fields.iter().position(|f| *f == s.field).unwrap_or(0);
                let n = fields.len();
                s.field = fields[if k == Key::Tab { (i + 1) % n } else { (i + n - 1) % n }];
                None
            }
            Key::Up | Key::Down => {
                if n_nets == 0 {
                    return None;
                }
                let i = match (s.selected, k) {
                    (None, _) => 0,
                    (Some(i), Key::Up) => i.saturating_sub(1),
                    (Some(i), _) => (i + 1).min(n_nets - 1),
                };
                Some(Action::PickNet(i as u8))
            }
            Key::Enter => Some(Action::Connect),
            Key::Backspace => {
                s.text(s.field).pop();
                None
            }
            Key::Char(c) => {
                let field = s.field;
                let (max, ok) = Settings::accepts(field, c);
                if !ok {
                    return None;
                }
                let text = s.text(field);
                if text.chars().count() < max {
                    text.push(c);
                }
                if field == Field::Network {
                    s.selected = None;
                }
                None
            }
            _ => None,
        }
    }

    /// Keys in the ATU-100 settings: arrows choose a cell, typing goes to
    /// its value, Enter sends it
    fn atu_key(&mut self, k: Key) -> Option<Action> {
        let a = self.app.atu.as_mut()?;
        match k {
            Key::Esc | Key::Char('e') => Some(Action::AtuClose), // values are numbers: no 'e' in them
            Key::Up | Key::BackTab => {
                a.scroll = None; // the list follows the keys again
                Some(Action::AtuPick(a.selected.saturating_sub(1) as u8))
            }
            Key::Down | Key::Tab => {
                a.scroll = None;
                Some(Action::AtuPick((a.selected + 1).min(CELLS.len() - 1) as u8))
            }
            Key::Enter => Some(Action::AtuSet),
            Key::Backspace => {
                a.edit.pop();
                None
            }
            Key::Char(c) => {
                let (max, ok) = Settings::accepts(Field::AtuValue, c);
                if ok && a.edit.chars().count() < max && cells::typed(&CELLS[a.selected]) {
                    a.edit.push(c);
                }
                None
            }
            _ => None,
        }
    }

    /// Keys on the connect screen: arrows choose, typing goes to the address
    fn connect_key(&mut self, k: Key) -> Option<Action> {
        let linked = !self.app.port.is_empty();
        let c = self.app.connect.as_mut()?;
        match k {
            Key::Esc if linked => Some(Action::CloseConnect),
            Key::Esc => Some(Action::Quit),
            Key::F(5) => Some(Action::ConnectRefresh),
            Key::Enter => Some(Action::ConnectGo),
            Key::Tab | Key::BackTab => {
                c.typing = !c.typing;
                None
            }
            Key::Up | Key::Down => {
                c.typing = false;
                let n = c.entries.len();
                if n > 0 {
                    c.selected = if k == Key::Up { c.selected.saturating_sub(1) } else { (c.selected + 1).min(n - 1) };
                }
                None
            }
            Key::Backspace => {
                c.address.pop();
                c.typing = true;
                None
            }
            Key::Char(ch) => {
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
    pub fn act(&mut self, action: Action) -> Option<Effect> {
        let (t, out, app) = (&mut self.t, &mut self.out, &mut self.app);
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
                app.atu = None;
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
                if esp.present == Some(true) && esp.may_set_wifi && esp.networks.is_empty() {
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
                let s = app.settings.as_mut()?;
                if s.ssid.is_empty() {
                    app.esp.notice = Some(("pick a network from the list, or type its name".into(), true));
                    return None;
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
                        return None;
                    };
                    let gateway = if s.gateway.is_empty() {
                        let o = ip.octets();
                        format!("{}.{}.{}.1", o[0], o[1], o[2]) // the usual router address
                    } else {
                        s.gateway.clone()
                    };
                    if gateway.parse::<std::net::Ipv4Addr>().is_err() {
                        app.esp.notice = Some(("the gateway address is not complete".into(), true));
                        return None;
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
            Action::HotspotAlways => {
                let always = !app.esp.hotspot_always;
                app.esp.hotspot_always = always;
                out.push(format!(r#"{{"ESP32": "Hotspot", "Always": {always}}}"#));
            }
            Action::UseWifi => match app.esp.wifi_port() {
                Some(p) => return Some(Effect::SwitchTo(p)),
                None => app.esp.notice = Some(("the bridge is not on Wi-Fi yet".into(), true)),
            },
            Action::ChangeLink => {
                app.settings = None;
                return Some(Effect::OpenConnect);
            }
            Action::CloseConnect => app.connect = None,
            Action::ConnectRefresh => return Some(Effect::ConnectRefresh),
            Action::ConnectPick(i) => return Some(Effect::ConnectPick(i)),
            Action::ConnectType => {
                if let Some(c) = app.connect.as_mut() {
                    c.typing = true;
                }
            }
            Action::ConnectGo => return Some(Effect::ConnectGo),
            Action::OpenWeb => {
                let url = app.web_reach.clone().flatten().or_else(|| app.esp.web_urls().into_iter().next())?;
                return Some(Effect::OpenWeb(url));
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
                let d = app.winding.take()?;
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
            Action::AtuOpen => {
                app.settings = None;
                let first = 0;
                app.atu = Some(AtuBox {
                    selected: first,
                    edit: cells::edit_text(&CELLS[first], t),
                    confirm_reset: false,
                    notice: None,
                    scroll: None,
                    top_drawn: std::cell::Cell::new(0),
                });
                if !t.dumped {
                    out.push(r#"{"Dump": "EEPROM"}"#.into());
                }
            }
            Action::AtuClose => app.atu = None,
            Action::AtuScroll(top) => {
                if let Some(a) = app.atu.as_mut() {
                    a.scroll = Some(top as usize);
                }
            }
            Action::AtuPick(i) => {
                let a = app.atu.as_mut()?;
                let i = (i as usize).min(CELLS.len() - 1);
                if i != a.selected || a.edit.is_empty() {
                    a.selected = i;
                    a.edit = cells::edit_text(&CELLS[i], t);
                    a.confirm_reset = false;
                    a.notice = None;
                }
            }
            Action::AtuChoose(v) => {
                let a = app.atu.as_mut()?;
                let d = &CELLS[a.selected];
                if matches!(d.kind, CellKind::Toggle { .. } | CellKind::Choice(_)) {
                    t.eeprom[d.cell as usize] = Some(v);
                    out.push(format!(r#"{{"Set": {}, "Value": {v}}}"#, d.cell));
                    a.notice = Some((format!("{} set", d.name), false));
                }
            }
            Action::AtuSet => {
                let a = app.atu.as_mut()?;
                let d = &CELLS[a.selected];
                match cells::encode(d, &a.edit) {
                    Ok(bytes) => {
                        for (cell, b) in bytes {
                            t.eeprom[cell as usize] = Some(b);
                            out.push(format!(r#"{{"Set": {cell}, "Value": {b}}}"#));
                        }
                        a.notice = Some((format!("{} set to {}", d.name, cells::shown(d, t)), false));
                    }
                    Err(e) => a.notice = Some((format!("{}: {e}", d.name), true)),
                }
            }
            Action::AtuReset => {
                let a = app.atu.as_mut()?;
                if !a.confirm_reset {
                    a.confirm_reset = true;
                    a.notice = Some(("Tap again to set every cell to the ATU-100 EXT defaults (the display cells stay as they are)".into(), true));
                } else {
                    for (cell, b) in cells::defaults() {
                        t.eeprom[cell as usize] = Some(b);
                        out.push(format!(r#"{{"Set": {cell}, "Value": {b}}}"#));
                    }
                    a.confirm_reset = false;
                    a.edit = cells::edit_text(&CELLS[a.selected], t);
                    a.notice = Some(("ATU-100 EXT defaults sent".into(), false));
                    t.log("ATU-100 settings set to the ATU-100 EXT defaults", Kind::Warn);
                }
            }
            Action::Quit => return Some(Effect::Quit),
        }
        None
    }
}
