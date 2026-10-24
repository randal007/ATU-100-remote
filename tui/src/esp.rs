//! The ESP32 bridge, when there is one between us and the tuner. Its own
//! messages carry an "ESP32" field; it answers them itself and never passes
//! them to the tuner. Without a bridge the tuner ignores them.

use serde_json::{Map, Value};
use std::time::Duration;
use web_time::Instant;

pub struct Net {
    pub ssid: String,
    pub rssi: i64,
    pub secure: bool,
}

#[derive(Default)]
pub struct Esp {
    /// None until the probe is answered or times out
    pub present: Option<bool>,
    probed_at: Option<Instant>,
    pub version: String,
    pub over_usb: bool,
    /// Wi-Fi settings may change from here: over USB, or from the bridge's
    /// own hotspot
    pub may_set_wifi: bool,
    pub wifi: String, // Off, Connecting, Connected, Failed
    pub ssid: String,
    pub ip: String,
    pub rssi: i64,
    pub host: String,
    pub port: u16,
    pub hotspot: bool,
    pub hotspot_name: String,
    pub hotspot_ip: String,
    pub hotspot_always: bool,
    pub web: bool, // its web page is being served
    pub web_port: u16,
    pub static_ip: String, // empty: the router gives the address (DHCP)
    pub gateway: String,
    pub mask: String,
    pub networks: Vec<Net>,
    found: Vec<Net>, // a scan coming in
    pub scanning: bool,
    scan_since: Option<Instant>,
    /// Last answer to a command: text, and whether it was an error
    pub notice: Option<(String, bool)>,
}

pub const PROBE: &str = r#"{"ESP32": "Info"}"#;

impl Esp {
    /// We have just asked whether a bridge is there
    pub fn probing(&mut self) {
        self.probed_at = Some(Instant::now());
    }

    /// Nothing answered the probe in time: no bridge on this link. And a
    /// scan whose answer never came (lost, or another client's scan took
    /// it) must not leave the scan button off for good
    pub fn check_probe(&mut self) {
        if self.present.is_none() && self.probed_at.is_some_and(|t| t.elapsed() > Duration::from_secs(3)) {
            self.present = Some(false);
        }
        if self.scanning && self.scan_since.is_some_and(|t| t.elapsed() > Duration::from_secs(20)) {
            self.scanning = false;
        }
    }

    /// A scan asked for
    pub fn start_scan(&mut self) {
        self.scanning = true;
        self.scan_since = Some(Instant::now());
    }

    /// Start again after the link changed
    pub fn forget_link(&mut self) {
        *self = Esp::default();
    }

    /// Handle a message if it is the bridge's; false if it is the tuner's.
    pub fn line(&mut self, msg: &Map<String, Value>) -> bool {
        let Some(kind) = msg.get("ESP32").and_then(|v| v.as_str()) else {
            return false;
        };
        let s = |name: &str| msg.get(name).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let i = |name: &str| msg.get(name).and_then(|v| v.as_i64()).unwrap_or(0);
        self.present = Some(true);
        match kind {
            "Info" => {
                self.version = s("Version");
                self.over_usb = s("Client") == "USB";
                self.may_set_wifi = matches!(s("Client").as_str(), "USB" | "Hotspot");
                self.wifi = s("WiFi");
                self.ssid = s("SSID");
                self.ip = s("IP");
                self.rssi = i("RSSI");
                self.host = s("Host");
                self.port = i("Port") as u16;
                self.hotspot = msg.get("Hotspot").and_then(|v| v.as_bool()).unwrap_or(false);
                self.hotspot_name = s("HotspotName");
                self.hotspot_ip = s("HotspotIP");
                self.hotspot_always = msg.get("HotspotAlways").and_then(|v| v.as_bool()).unwrap_or(false);
                self.web = msg.get("Web").and_then(|v| v.as_bool()).unwrap_or(false);
                self.web_port = i("WebPort") as u16;
                self.static_ip = s("StaticIP");
                self.gateway = s("Gateway");
                self.mask = s("Mask");
            }
            "Net" => self.found.push(Net {
                ssid: s("SSID"),
                rssi: i("RSSI"),
                secure: msg.get("Secure").and_then(|v| v.as_bool()).unwrap_or(true),
            }),
            "Scan" => {
                let mut found = std::mem::take(&mut self.found);
                // one line per name, the strongest, then strongest first
                found.sort_by(|a, b| a.ssid.cmp(&b.ssid).then(b.rssi.cmp(&a.rssi)));
                found.dedup_by(|a, b| a.ssid == b.ssid);
                found.retain(|n| !n.ssid.is_empty());
                found.sort_by_key(|n| -n.rssi);
                self.networks = found;
                self.scanning = false;
            }
            "OK" => self.notice = Some((s("Text"), false)),
            "Error" => {
                self.notice = Some((s("Text"), true));
                self.scanning = false;
            }
            _ => {}
        }
        true
    }

    /// Where the bridge's web page is served: its address on the home
    /// network, and on its own hotspot
    pub fn web_urls(&self) -> Vec<String> {
        let url = |ip: &str| match self.web_port {
            0 | 80 => format!("http://{ip}/"),
            p => format!("http://{ip}:{p}/"),
        };
        let mut list = vec![];
        if self.web && self.wifi == "Connected" && !self.ip.is_empty() {
            list.push(url(&self.ip));
        }
        if self.web && self.hotspot && !self.hotspot_ip.is_empty() {
            list.push(url(&self.hotspot_ip));
        }
        list
    }

    /// Where to reach the bridge over Wi-Fi, once it is on the network
    pub fn wifi_port(&self) -> Option<String> {
        (self.wifi == "Connected" && !self.ip.is_empty()).then(|| format!("socket://{}:{}", self.ip, self.port))
    }
}

/// Signal strength as bars
pub fn bars(rssi: i64) -> &'static str {
    match rssi {
        r if r >= -55 => "▂▄▆█",
        r if r >= -67 => "▂▄▆ ",
        r if r >= -78 => "▂▄  ",
        _ => "▂   ",
    }
}
