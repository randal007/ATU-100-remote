//! The tuner's EEPROM settings, the ones N7DDC's manual describes and the
//! ones this firmware adds: what each cell means, how its byte reads and
//! how a typed value becomes bytes again. The same cells a PICkit sets; here
//! they change over the serial link and take effect at once.

use crate::tuner::Tuner;

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    /// BCD, read as written: 15 = 15. `tenths`: one decimal, 13 = 1.3.
    /// `zero`: what 0 means, when it means something
    Bcd { tenths: bool, unit: &'static str, zero: Option<&'static str>, ff: Option<&'static str> },
    /// Two cells, four BCD digits: 01 10 = 110 (nH or pF)
    Pair { unit: &'static str },
    /// 0 or 1
    Toggle { off: &'static str, on: &'static str },
    /// One of a few values
    Choice(&'static [(u8, &'static str)]),
    /// A plain byte, 0 to 99
    Raw { unit: &'static str },
    /// The detector winding: changed only through its own warning box
    Winding,
}

pub struct Def {
    pub cell: u8,
    pub name: &'static str,
    pub kind: Kind,
    /// What this firmware's hex starts with: N7DDC's ATU-100 EXT values,
    /// an OLED for the display, and this firmware's own cells
    pub default: u16,
    /// Left alone by "defaults": a wrong display setting blanks the screen,
    /// and the detector winding (and "Tunes from" with it) must match the
    /// transformer, so it changes only through its own warning box
    pub keep: bool,
    pub note: &'static str,
}

pub enum Item {
    Group(&'static str),
    Cell(usize), // index into CELLS
}

const DISPLAYS: &[(u8, &str)] = &[
    (0, "two-colour LED"),
    (1, "1602 LCD"),
    (2, "128×32 OLED"),
    (3, "128×32 OLED, inverted"),
    (4, "128×64 OLED"),
    (5, "128×64 OLED, inverted"),
];
const ADDRESSES: &[(u8, &str)] = &[(0x4E, "4E (1602 LCD)"), (0x7E, "7E (1602 LCD)"), (0x78, "78 (OLED)"), (0x7A, "7A (OLED)")];
const COUNTS: &[(u8, &str)] = &[(5, "5"), (6, "6"), (7, "7")];

const fn bcd(tenths: bool, unit: &'static str) -> Kind {
    Kind::Bcd { tenths, unit, zero: None, ff: None }
}

pub const CELLS: &[Def] = &[
    // Tuning
    Def { cell: 0x03, name: "Relay settling time", kind: bcd(false, "ms"), default: 0x15, keep: false, note: "wait after switching relays before measuring" },
    Def { cell: 0x04, name: "Auto tunes above SWR", kind: bcd(true, ""), default: 0x13, keep: false, note: "in Auto mode" },
    Def { cell: 0x05, name: "Tunes from", kind: bcd(false, "W"), default: 0x05, keep: true, note: "minimum power to tune; tens of W in high power mode" },
    Def { cell: 0x06, name: "Tunes up to", kind: Kind::Bcd { tenths: false, unit: "W", zero: Some("no limit"), ff: None }, default: 0x00, keep: false, note: "maximum power to tune; tens of W in high power mode" },
    Def { cell: 0x09, name: "Tunes only below SWR", kind: Kind::Bcd { tenths: true, unit: "", zero: Some("always tunes"), ff: None }, default: 0x00, keep: false, note: "no tune if the SWR starts above this" },
    // Power and detector
    Def { cell: 0x31, name: "Detector winding", kind: Kind::Winding, default: 0x10, keep: true, note: "tandem match turns ratio 1:N; must match the transformer" },
    Def { cell: 0x30, name: "High power mode", kind: Kind::Toggle { off: "off (to 150 W)", on: "on (to 9999 W)" }, default: 0x00, keep: false, note: "needs a matching tandem match winding" },
    Def { cell: 0x0E, name: "Diode correction", kind: Kind::Toggle { off: "off", on: "on" }, default: 0x01, keep: false, note: "software correction of the detector diodes" },
    Def { cell: 0x37, name: "HighSWR alarm", kind: Kind::Bcd { tenths: true, unit: "", zero: Some("off"), ff: Some("3.0 (unset)") }, default: 0xFF, keep: false, note: "HighSWR event at or above this SWR" },
    Def { cell: 0x33, name: "Antenna power, efficiency", kind: Kind::Toggle { off: "not shown", on: "shown" }, default: 0x01, keep: false, note: "ANT and EFF on the display and in the status" },
    Def { cell: 0x34, name: "Feeder loss", kind: bcd(true, "dB"), default: 0x12, keep: false, note: "for the antenna power" },
    // Display
    Def { cell: 0x00, name: "Display I2C address", kind: Kind::Choice(ADDRESSES), default: 0x78, keep: true, note: "wrong: the display stays blank" },
    Def { cell: 0x01, name: "Display type", kind: Kind::Choice(DISPLAYS), default: 0x05, keep: true, note: "wrong: the display stays blank" },
    Def { cell: 0x07, name: "OLED rows down", kind: Kind::Raw { unit: "" }, default: 0x00, keep: true, note: "picture position on an OLED" },
    Def { cell: 0x08, name: "OLED columns left", kind: Kind::Raw { unit: "" }, default: 0x02, keep: true, note: "picture position on an OLED" },
    Def { cell: 0x32, name: "Display off after", kind: Kind::Bcd { tenths: false, unit: "s", zero: Some("always on"), ff: None }, default: 0x00, keep: false, note: "RF or a button turns it on again" },
    Def { cell: 0x35, name: "Relays off with the display", kind: Kind::Toggle { off: "no", on: "yes" }, default: 0x00, keep: false, note: "releases the relays while the display is off" },
    // Relays
    Def { cell: 0x0A, name: "Number of inductors", kind: Kind::Choice(COUNTS), default: 0x07, keep: false, note: "5 to 7" },
    Def { cell: 0x0B, name: "Inductor steps", kind: Kind::Toggle { off: "binary", on: "linear" }, default: 0x00, keep: false, note: "linear: all inductors the same value" },
    Def { cell: 0x0C, name: "Number of capacitors", kind: Kind::Choice(COUNTS), default: 0x07, keep: false, note: "5 to 7" },
    Def { cell: 0x0D, name: "Capacitor steps", kind: Kind::Toggle { off: "binary", on: "linear" }, default: 0x00, keep: false, note: "linear: all capacitors the same value" },
    Def { cell: 0x0F, name: "Inductor relays", kind: Kind::Toggle { off: "normally closed", on: "normally open" }, default: 0x00, keep: false, note: "which contacts short the inductors" },
    // Inductors and capacitors, as built or as measured
    Def { cell: 0x10, name: "L1", kind: Kind::Pair { unit: "nH" }, default: 50, keep: false, note: "smallest inductor; type what you measured" },
    Def { cell: 0x12, name: "L2", kind: Kind::Pair { unit: "nH" }, default: 110, keep: false, note: "" },
    Def { cell: 0x14, name: "L3", kind: Kind::Pair { unit: "nH" }, default: 220, keep: false, note: "" },
    Def { cell: 0x16, name: "L4", kind: Kind::Pair { unit: "nH" }, default: 450, keep: false, note: "" },
    Def { cell: 0x18, name: "L5", kind: Kind::Pair { unit: "nH" }, default: 1000, keep: false, note: "" },
    Def { cell: 0x1A, name: "L6", kind: Kind::Pair { unit: "nH" }, default: 2200, keep: false, note: "" },
    Def { cell: 0x1C, name: "L7", kind: Kind::Pair { unit: "nH" }, default: 4500, keep: false, note: "largest inductor" },
    Def { cell: 0x20, name: "C1", kind: Kind::Pair { unit: "pF" }, default: 10, keep: false, note: "smallest capacitor; type what you measured" },
    Def { cell: 0x22, name: "C2", kind: Kind::Pair { unit: "pF" }, default: 22, keep: false, note: "" },
    Def { cell: 0x24, name: "C3", kind: Kind::Pair { unit: "pF" }, default: 47, keep: false, note: "" },
    Def { cell: 0x26, name: "C4", kind: Kind::Pair { unit: "pF" }, default: 100, keep: false, note: "" },
    Def { cell: 0x28, name: "C5", kind: Kind::Pair { unit: "pF" }, default: 220, keep: false, note: "" },
    Def { cell: 0x2A, name: "C6", kind: Kind::Pair { unit: "pF" }, default: 470, keep: false, note: "" },
    Def { cell: 0x2C, name: "C7", kind: Kind::Pair { unit: "pF" }, default: 1000, keep: false, note: "largest capacitor" },
];

/// The rows of the settings box, with group headings
pub fn items() -> Vec<Item> {
    let groups: [(&str, u8); 6] = [
        ("Tuning", 0x03),
        ("Power and detector", 0x31),
        ("Display", 0x00),
        ("Relays", 0x0A),
        ("Inductors", 0x10),
        ("Capacitors", 0x20),
    ];
    let mut list = vec![];
    for (i, d) in CELLS.iter().enumerate() {
        if let Some((name, _)) = groups.iter().find(|(_, first)| *first == d.cell) {
            list.push(Item::Group(name));
        }
        list.push(Item::Cell(i));
    }
    list
}

fn from_bcd(b: u8) -> Option<u16> {
    let (hi, lo) = (b >> 4, b & 15);
    (hi < 10 && lo < 10).then_some(hi as u16 * 10 + lo as u16)
}

fn to_bcd(n: u16) -> u8 {
    ((n / 10) << 4 | n % 10) as u8
}

/// The cell's raw bytes, as a PICkit shows them: "15", "01 10"
pub fn raw(d: &Def, t: &Tuner) -> String {
    let byte = |c: u8| t.eeprom[c as usize].map_or("..".into(), |b| format!("{b:02X}"));
    match d.kind {
        Kind::Pair { .. } => format!("{} {}", byte(d.cell), byte(d.cell + 1)),
        _ => byte(d.cell),
    }
}

/// The value in plain units, "…" until the tuner has said
pub fn shown(d: &Def, t: &Tuner) -> String {
    let Some(b) = t.eeprom[d.cell as usize] else { return "…".into() };
    match d.kind {
        Kind::Bcd { tenths, unit, zero, ff } => {
            if b == 0xFF {
                if let Some(f) = ff {
                    return f.into();
                }
            }
            match (from_bcd(b), zero) {
                (Some(0), Some(z)) => z.into(),
                (Some(n), Some(z)) if d.cell == 0x09 && n <= 11 => z.into(), // the firmware ignores up to 1.1
                (Some(n), _) if tenths => format!("{}.{} {unit}", n / 10, n % 10).trim_end().into(),
                (Some(n), _) => format!("{n} {unit}").trim_end().into(),
                (None, _) => format!("not valid ({b:02X})"),
            }
        }
        Kind::Pair { unit } => match (from_bcd(b), t.eeprom[d.cell as usize + 1].and_then(from_bcd)) {
            (Some(h), Some(l)) => format!("{} {unit}", h * 100 + l),
            _ => "not valid".into(),
        },
        Kind::Toggle { off, on } => match b {
            0 => off.into(),
            1 => on.into(),
            _ => format!("not valid ({b:02X})"),
        },
        Kind::Choice(list) => list.iter().find(|(v, _)| *v == b).map_or(format!("{b:02X}"), |(_, n)| n.to_string()),
        Kind::Raw { unit } => format!("{b} {unit}").trim_end().into(),
        Kind::Winding => from_bcd(b).map_or(format!("{b:02X}"), |n| format!("1:{n}")),
    }
}

/// The value as typed into the editor, from the cell
pub fn edit_text(d: &Def, t: &Tuner) -> String {
    let Some(b) = t.eeprom[d.cell as usize] else { return String::new() };
    match d.kind {
        Kind::Bcd { tenths, .. } => match from_bcd(b) {
            Some(n) if tenths => format!("{}.{}", n / 10, n % 10),
            Some(n) => n.to_string(),
            None => String::new(),
        },
        Kind::Pair { .. } => match (from_bcd(b), t.eeprom[d.cell as usize + 1].and_then(from_bcd)) {
            (Some(h), Some(l)) => (h * 100 + l).to_string(),
            _ => String::new(),
        },
        Kind::Raw { .. } => b.to_string(),
        _ => String::new(),
    }
}

/// Is this a value typed in (a number), rather than picked?
pub fn typed(d: &Def) -> bool {
    matches!(d.kind, Kind::Bcd { .. } | Kind::Pair { .. } | Kind::Raw { .. })
}

/// What a typed value writes: (cell, byte) pairs, or what is wrong with it
fn is_swr(d: &Def) -> bool {
    matches!(d.kind, Kind::Bcd { tenths: true, unit: "", .. })
}

fn zero_means(d: &Def) -> bool {
    matches!(d.kind, Kind::Bcd { zero: Some(_), .. })
}

pub fn encode(d: &Def, text: &str) -> Result<Vec<(u8, u8)>, String> {
    let text = text.trim();
    match d.kind {
        Kind::Bcd { tenths, .. } => {
            let v: f64 = text.parse().map_err(|_| "type a number".to_string())?;
            let n = if tenths { (v * 10.0).round() } else { v.round() };
            let max = if tenths { "9.9" } else { "99" };
            if !(0.0..=99.0).contains(&n) {
                return Err(format!("0 to {max}"));
            }
            // An SWR (tenths, no unit): below 1.1 the alarm or auto tune would
            // fire all the time
            if is_swr(d) {
                if n < 11.0 && !(n == 0.0 && zero_means(d)) {
                    return Err(if zero_means(d) { "0, or 1.1 to 9.9".into() } else { "1.1 to 9.9".into() });
                }
            }
            if !tenths && v.fract() != 0.0 {
                return Err("a whole number".into());
            }
            Ok(vec![(d.cell, to_bcd(n as u16))])
        }
        Kind::Pair { unit } => {
            let n: u16 = text.parse().map_err(|_| format!("a whole number of {unit}"))?;
            if n > 9999 {
                return Err(format!("0 to 9999 {unit}"));
            }
            Ok(vec![(d.cell, to_bcd(n / 100)), (d.cell + 1, to_bcd(n % 100))])
        }
        Kind::Raw { .. } => {
            let n: u8 = text.parse().map_err(|_| "a whole number".to_string())?;
            if n > 99 {
                return Err("0 to 99".into());
            }
            Ok(vec![(d.cell, n)])
        }
        _ => Err("pick a value".into()),
    }
}

/// The bytes of the ATU-100 EXT defaults, display cells left alone
pub fn defaults() -> Vec<(u8, u8)> {
    let mut out = vec![];
    for d in CELLS.iter().filter(|d| !d.keep) {
        match d.kind {
            Kind::Pair { .. } => {
                out.push((d.cell, to_bcd(d.default / 100)));
                out.push((d.cell + 1, to_bcd(d.default % 100)));
            }
            _ => out.push((d.cell, d.default as u8)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_as_a_pickit_would() {
        let find = |c: u8| CELLS.iter().find(|d| d.cell == c).unwrap();
        assert_eq!(encode(find(0x04), "1.3"), Ok(vec![(0x04, 0x13)]));
        assert_eq!(encode(find(0x05), "5"), Ok(vec![(0x05, 0x05)]));
        assert_eq!(encode(find(0x34), "1.2"), Ok(vec![(0x34, 0x12)]));
        assert_eq!(encode(find(0x18), "1000"), Ok(vec![(0x18, 0x10), (0x19, 0x00)]));
        assert_eq!(encode(find(0x10), "53"), Ok(vec![(0x10, 0x00), (0x11, 0x53)]));
        assert_eq!(encode(find(0x2C), "1012"), Ok(vec![(0x2C, 0x10), (0x2D, 0x12)]));
        assert!(encode(find(0x05), "120").is_err());
        assert!(encode(find(0x10), "10000").is_err());
        assert!(encode(find(0x05), "2.5").is_err());
        // SWR limits: 0 where it means off, else 1.1 to 9.9; a loss can be 0
        assert_eq!(encode(find(0x37), "0"), Ok(vec![(0x37, 0x00)]));
        assert!(encode(find(0x37), "0.5").is_err());
        assert!(encode(find(0x04), "0").is_err());
        assert_eq!(encode(find(0x34), "0"), Ok(vec![(0x34, 0x00)]));
        // "defaults" leaves the display, the winding and Tunes from alone
        assert!(defaults().iter().all(|&(c, _)| ![0x00, 0x01, 0x05, 0x07, 0x08, 0x31].contains(&c)));
        let mut t = Tuner::new();
        t.eeprom[0x18] = Some(0x10);
        t.eeprom[0x19] = Some(0x00);
        assert_eq!(shown(find(0x18), &t), "1000 nH");
        assert_eq!(edit_text(find(0x18), &t), "1000");
        t.eeprom[0x04] = Some(0x13);
        assert_eq!(shown(find(0x04), &t), "1.3");
        t.eeprom[0x37] = Some(0xFF);
        assert_eq!(shown(find(0x37), &t), "3.0 (unset)");
        // defaults never touch the display cells
        assert!(defaults().iter().all(|(c, _)| ![0x00, 0x01, 0x07, 0x08].contains(c)));
    }
}
