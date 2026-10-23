//! What we know about the tuner, kept up to date from its messages.

use serde_json::Value;
use std::collections::VecDeque;
use std::time::Instant;

pub const HISTORY: usize = 600; // samples, one every 0.5 s: five minutes

// EEPROM cells used here
pub const CELL_AUTO_SWR: usize = 0x04;
pub const CELL_MIN_POWER: usize = 0x05;
pub const CELL_HIGH_POWER: usize = 0x30;
pub const CELL_WINDING: usize = 0x31; // tandem match turns ratio, BCD: 10 = 1:10
pub const CELL_TUNE_MODE: usize = 0x36;
pub const CELL_SWR_ALARM: usize = 0x37;
const CELL_LAST_SW: usize = 0xfd;
const CELL_LAST_IND: usize = 0xfe;
const CELL_LAST_CAP: usize = 0xff;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Info,
    Good,
    Warn,
    Bad,
}

pub struct Event {
    pub time: String,
    pub text: String,
    pub kind: Kind,
}

pub struct Tuner {
    pub auto: bool,
    pub bypass: bool,
    pub power: f64,
    pub antenna: Option<f64>,
    pub efficiency: Option<i64>,
    pub swr: f64,
    pub order_lc: bool, // capacitors on the antenna side
    pub cap_pf: i64,
    pub ind_nh: i64,
    pub fwd_mv: Option<i64>,
    pub rev_mv: Option<i64>,
    pub tuning: bool,
    pub high_swr: bool,
    pub high_power: bool,
    pub build: Option<String>,
    pub have_status: bool,
    pub eeprom: [Option<u8>; 256],
    pub ind_values: [i64; 7], // nH, from cells 10-1d
    pub cap_values: [i64; 7], // pF, from cells 20-2d
    pub values_from_tuner: bool,
    pub ind_bits: u8,
    pub cap_bits: u8,
    pub events: VecDeque<Event>,
    pub power_hist: VecDeque<f64>,
    pub swr_hist: VecDeque<f64>,
    pub last_rx: Option<Instant>,
    pub dumped: bool,
}

pub fn now() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

fn bcd(b: u8) -> i64 {
    (b >> 4) as i64 * 10 + (b & 15) as i64
}

impl Tuner {
    pub fn new() -> Tuner {
        Tuner {
            auto: false,
            bypass: false,
            power: 0.0,
            antenna: None,
            efficiency: None,
            swr: 0.0,
            order_lc: true,
            cap_pf: 0,
            ind_nh: 0,
            fwd_mv: None,
            rev_mv: None,
            tuning: false,
            high_swr: false,
            high_power: false,
            build: None,
            have_status: false,
            eeprom: [None; 256],
            // N7DDC's defaults until the tuner tells us
            ind_values: [50, 110, 220, 450, 1000, 2200, 4500],
            cap_values: [10, 22, 47, 100, 220, 470, 1000],
            values_from_tuner: false,
            ind_bits: 0,
            cap_bits: 0,
            events: VecDeque::new(),
            power_hist: VecDeque::new(),
            swr_hist: VecDeque::new(),
            last_rx: None,
            dumped: false,
        }
    }

    pub fn log(&mut self, text: impl Into<String>, kind: Kind) {
        self.events.push_front(Event { time: now(), text: text.into(), kind });
        self.events.truncate(200);
    }

    /// Record the current readings in the graphs.
    pub fn sample(&mut self) {
        self.power_hist.push_back(self.power);
        self.swr_hist.push_back(self.swr);
        while self.power_hist.len() > HISTORY {
            self.power_hist.pop_front();
            self.swr_hist.pop_front();
        }
    }

    pub fn line(&mut self, text: &str) {
        self.last_rx = Some(Instant::now());
        let Ok(Value::Object(msg)) = serde_json::from_str::<Value>(text) else {
            return; // not JSON: ignore
        };
        let mut cells = false;
        for (name, value) in &msg {
            match name.as_str() {
                "Auto" => self.auto = value.as_bool().unwrap_or(self.auto),
                "Bypass" => self.bypass = value.as_bool().unwrap_or(self.bypass),
                "Power" => {
                    self.power = value.as_f64().unwrap_or(0.0);
                    if let Some(limit) = self.power_alarm() {
                        if self.power < limit * 0.9 {
                            self.high_power = false;
                        }
                    }
                    self.have_status = true;
                    if !msg.contains_key("Antenna") {
                        self.antenna = None; // only sent while the tuner shows it
                        self.efficiency = None;
                    }
                }
                "Antenna" => self.antenna = value.as_f64(),
                "Efficiency" | "efficency" => self.efficiency = value.as_i64(),
                "SWR" => {
                    self.swr = value.as_f64().unwrap_or(0.0);
                    if let Some(limit) = self.alarm() {
                        if self.swr < limit - 0.1 {
                            self.high_swr = false;
                        }
                    }
                }
                "Order" => self.order_lc = value.as_str() == Some("LC"),
                "Capacitance" => {
                    self.cap_pf = value.as_i64().unwrap_or(0);
                    if let Some(bits) = relays_for(self.cap_pf, &self.cap_values) {
                        self.cap_bits = bits;
                    }
                }
                "Inductance" => {
                    self.ind_nh = value.as_i64().unwrap_or(0);
                    if let Some(bits) = relays_for(self.ind_nh, &self.ind_values) {
                        self.ind_bits = bits;
                    }
                }
                "Fwd" => self.fwd_mv = value.as_i64(),
                "Rev" => self.rev_mv = value.as_i64(),
                "Build" => self.build = value.as_str().map(String::from),
                "Event" => self.event(value.as_str().unwrap_or("?")),
                _ => {
                    // EEPROM cells: {"37":"30"}, or all 256 in a dump
                    if let (Ok(cell), Some(Ok(data))) = (
                        usize::from_str_radix(name, 16),
                        value.as_str().map(|v| u8::from_str_radix(v, 16)),
                    ) {
                        if name.len() == 2 && cell < 256 {
                            self.eeprom[cell] = Some(data);
                            cells = true;
                        }
                    }
                }
            }
        }
        if cells {
            self.cells_changed(msg.len() > 200);
        }
    }

    fn event(&mut self, event: &str) {
        let kind = match event {
            "Tune" => {
                self.tuning = true;
                Kind::Warn
            }
            "Tuned" => {
                self.tuning = false;
                Kind::Good
            }
            "Reset" | "Startup" | "Restart" => {
                self.tuning = false;
                Kind::Info
            }
            "HighSWR" => {
                self.high_swr = true;
                Kind::Bad
            }
            "HighPower" => {
                self.high_power = true;
                Kind::Bad
            }
            "Overload" | "Error" => Kind::Bad,
            "Busy" => Kind::Warn,
            _ => Kind::Info,
        };
        let text = match event {
            "Tune" => "Tune: transmit a carrier".to_string(),
            "Tuned" => "Tuned".to_string(),
            "HighSWR" => format!("High SWR ({:.2})", self.swr),
            "HighPower" => format!("High power ({:.0} W): more than the tuner is set up for", self.power),
            "Overload" => "Overload: more power than the detector can measure".to_string(),
            "Startup" => "Tuner started".to_string(),
            "Restart" => "Tuner restarted by its watchdog".to_string(),
            "Reset" => "Relays reset".to_string(),
            "Busy" => "Busy tuning: command not done".to_string(),
            other => other.to_string(),
        };
        self.log(text, kind);
    }

    fn cells_changed(&mut self, dump: bool) {
        let e = &self.eeprom;
        let pair = |a: usize| match (e[a], e[a + 1]) {
            (Some(h), Some(l)) => Some(bcd(h) * 100 + bcd(l)),
            _ => None,
        };
        let ind: Option<Vec<i64>> = (0..7).map(|i| pair(0x10 + 2 * i)).collect();
        let cap: Option<Vec<i64>> = (0..7).map(|i| pair(0x20 + 2 * i)).collect();
        if let (Some(ind), Some(cap)) = (ind, cap) {
            self.ind_values.copy_from_slice(&ind);
            self.cap_values.copy_from_slice(&cap);
            self.values_from_tuner = true;
        }
        if dump {
            self.dumped = true;
            // relays as last saved, until a status line says otherwise
            if let (Some(l), Some(c), Some(sw)) = (e[CELL_LAST_IND], e[CELL_LAST_CAP], e[CELL_LAST_SW]) {
                if !self.have_status {
                    self.ind_bits = l & 0x7f;
                    self.cap_bits = c & 0x7f;
                    self.order_lc = sw == 1;
                }
            }
            if let Some(bits) = relays_for(self.ind_nh, &self.ind_values) {
                self.ind_bits = bits;
            }
            if let Some(bits) = relays_for(self.cap_pf, &self.cap_values) {
                self.cap_bits = bits;
            }
        }
    }

    /// HighSWR limit; None when off. Unknown or unset cells mean the default 3.0.
    pub fn alarm(&self) -> Option<f64> {
        match self.eeprom[CELL_SWR_ALARM] {
            None | Some(0xff) => Some(3.0),
            Some(0) => None,
            Some(b) => Some(bcd(b) as f64 / 10.0),
        }
    }

    pub fn search_original(&self) -> Option<bool> {
        self.eeprom[CELL_TUNE_MODE].map(|b| b == 0)
    }

    pub fn high_power(&self) -> bool {
        self.eeprom[CELL_HIGH_POWER] == Some(1)
    }

    /// Tandem match turns ratio, 10 for 1:10
    pub fn winding(&self) -> Option<i64> {
        self.eeprom[CELL_WINDING].map(bcd)
    }

    /// The firmware's HighPower limit in W (as in its cells_init)
    pub fn power_alarm(&self) -> Option<f64> {
        let k = self.winding()?;
        if self.high_power() {
            None
        } else if k >= 9 {
            Some(100.0)
        } else {
            Some(1.4 * (k * k) as f64)
        }
    }

    /// Power meter scale: 0-100 W for the stock winding, 0-40 W for 1:5
    pub fn power_scale(&self) -> Option<f64> {
        let k = self.winding()?;
        if self.high_power() {
            None
        } else if k >= 9 {
            Some(100.0)
        } else {
            Some((1.6 * (k * k) as f64).round())
        }
    }

    pub fn auto_swr(&self) -> Option<f64> {
        self.eeprom[CELL_AUTO_SWR].map(|b| bcd(b) as f64 / 10.0)
    }

    pub fn min_power(&self) -> Option<i64> {
        self.eeprom[CELL_MIN_POWER].map(|b| bcd(b) * if self.high_power() { 10 } else { 1 })
    }
}

/// Which relays add up to a total. Each relay value is larger than all the
/// smaller ones together, so taking the largest that fits gives the only answer.
pub fn relays_for(mut total: i64, values: &[i64; 7]) -> Option<u8> {
    let mut bits = 0u8;
    for relay in (0..7).rev() {
        if values[relay] > 0 && values[relay] <= total {
            bits |= 1 << relay;
            total -= values[relay];
        }
    }
    (total == 0).then_some(bits)
}

/// BCD byte for an SWR alarm limit: 2.5 -> 0x25
pub fn alarm_bcd(swr: f64) -> u8 {
    let tenths = (swr * 10.0).round() as u8;
    (tenths / 10) << 4 | tenths % 10
}
