//! Drawing, in the style of btop: rounded boxes, gradient meters and braille
//! graphs, with btop's default theme colours.

use crate::esp::bars;
use crate::tuner::{Kind, Tuner};
use crate::app::{App, Field};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use ratatui::Frame;

const MAIN_FG: Color = Color::Rgb(0xcc, 0xcc, 0xcc);
const TITLE: Color = Color::Rgb(0xee, 0xee, 0xee);
const HI: Color = Color::Rgb(0xb5, 0x40, 0x40);
const INACTIVE: Color = Color::Rgb(0x50, 0x50, 0x50);
const GRAPH_TEXT: Color = Color::Rgb(0x70, 0x70, 0x70);
const METER_BG: Color = Color::Rgb(0x3a, 0x3a, 0x3a);
const DIV: Color = Color::Rgb(0x30, 0x30, 0x30);
const BOX_POWER: Color = Color::Rgb(0x3d, 0x7b, 0x46);
const BOX_SWR: Color = Color::Rgb(0x8a, 0x88, 0x2e);
const BOX_TUNER: Color = Color::Rgb(0x42, 0x3b, 0xa5);
const BOX_RELAYS: Color = Color::Rgb(0x92, 0x35, 0x35);
const BOX_EVENTS: Color = Color::Rgb(0x5a, 0x5a, 0x5a);
const GREEN: Color = Color::Rgb(0x50, 0xf0, 0x95);
const YELLOW: Color = Color::Rgb(0xf2, 0xe2, 0x66);
const RED: Color = Color::Rgb(0xfa, 0x1e, 0x1e);
const ON_BG: Color = Color::Rgb(0x2d, 0x6b, 0x3c);
const BOX_SETTINGS: Color = Color::Rgb(0x2e, 0x7d, 0x8c);
const FIELD_BG: Color = Color::Rgb(0x26, 0x26, 0x26);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    Tune,
    Bypass,
    Auto,
    Reset,
    Status,
    Detector,
    Search,
    AlarmUp,
    AlarmDown,
    Ind(u8),
    Cap(u8),
    Side,
    Settings,
    CloseSettings,
    Scan,
    PickNet(u8),
    Focus(Field),
    AddrFixed(bool),
    ShowPass,
    Connect,
    Forget,
    UseWifi,
    ChangeLink,
    CloseConnect,
    ConnectRefresh,
    ConnectPick(u8),
    ConnectType,
    ConnectGo,
    OpenWeb,
    HotspotAlways,
    MinToggle,
    WindingOpen,
    WindingChoose(u8),
    WindingSetMin,
    WindingApply,
    WindingCancel,
    Quit,
}

/// Screen areas the mouse can click, filled in while drawing.
#[derive(Default)]
pub struct Hits {
    pub areas: Vec<(Rect, Action)>,
    pub alarm_row: Option<Rect>, // scroll wheel here changes the limit
}

impl Hits {
    pub fn at(&self, x: u16, y: u16) -> Option<Action> {
        self.areas
            .iter()
            .find(|(r, _)| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
            .map(|(_, a)| *a)
    }
}

/// btop's cpu gradient: green, yellow, red
fn gradient(t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    let (a, b, u) = if t < 0.5 { (GREEN, YELLOW, t * 2.0) } else { (YELLOW, RED, t * 2.0 - 1.0) };
    let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (a, b) else { return a };
    let mix = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * u).round() as u8;
    Color::Rgb(mix(r1, r2), mix(g1, g2), mix(b1, b2))
}

fn swr_color(swr: f64) -> Color {
    if swr == 0.0 { MAIN_FG } else { gradient((swr - 1.0) / 2.0) }
}

fn boxed<'a>(number: &'a str, name: &'a str, color: Color) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(color))
        .title(Line::from(vec![
            Span::styled("─", Style::new().fg(color)),
            Span::styled(number, Style::new().fg(HI)),
            Span::styled(name, Style::new().fg(TITLE).add_modifier(Modifier::BOLD)),
            Span::styled("─", Style::new().fg(color)),
        ]))
}

/// Text in the top border on the right, between dashes as btop does
fn corner<'a>(text: String, color: Color) -> Line<'a> {
    Line::from(vec![
        Span::styled("─", Style::new().fg(color)),
        Span::styled(text, Style::new().fg(GRAPH_TEXT)),
        Span::styled("─", Style::new().fg(color)),
    ])
    .right_aligned()
}

/// A btop meter: one ■ per cell, coloured along the gradient up to the value.
fn meter(buf: &mut Buffer, area: Rect, frac: f64) {
    let w = area.width as usize;
    if w == 0 {
        return;
    }
    let filled = (frac.clamp(0.0, 1.0) * w as f64).round() as usize;
    for i in 0..w {
        let color = if i < filled { gradient(i as f64 / (w.max(2) - 1) as f64) } else { METER_BG };
        if let Some(cell) = buf.cell_mut((area.x + i as u16, area.y)) {
            cell.set_symbol("■").set_fg(color);
        }
    }
}

/// A btop braille graph: two samples per cell, four dots per row, filled
/// from the bottom, coloured by height. Newest sample on the right.
fn graph(buf: &mut Buffer, area: Rect, samples: impl DoubleEndedIterator<Item = f64>) {
    const LEFT: [u32; 4] = [0x40, 0x04, 0x02, 0x01]; // bottom to top
    const RIGHT: [u32; 4] = [0x80, 0x20, 0x10, 0x08];
    let (w, h) = (area.width as usize, area.height as usize);
    if w == 0 || h == 0 {
        return;
    }
    let mut values: Vec<f64> = samples.rev().take(w * 2).collect();
    values.resize(w * 2, 0.0);
    values.reverse();
    let dots = (h * 4) as f64;
    for col in 0..w {
        let left = (values[col * 2].clamp(0.0, 1.0) * dots).round() as i64;
        let right = (values[col * 2 + 1].clamp(0.0, 1.0) * dots).round() as i64;
        for row in 0..h {
            let base = row as i64 * 4;
            let mut bits = 0u32;
            for d in 0..4 {
                if left > base + d as i64 {
                    bits |= LEFT[d];
                }
                if right > base + d as i64 {
                    bits |= RIGHT[d];
                }
            }
            if bits == 0 {
                continue;
            }
            let ch = char::from_u32(0x2800 + bits).unwrap_or(' ');
            let color = gradient((row as f64 + 0.5) / h as f64);
            let y = area.y + (h - 1 - row) as u16;
            if let Some(cell) = buf.cell_mut((area.x + col as u16, y)) {
                cell.set_char(ch).set_fg(color);
            }
        }
    }
}

/// A round number at or above v for a graph scale
fn nice_max(v: f64) -> f64 {
    for m in [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 150.0, 200.0, 500.0, 1000.0, 1500.0, 2000.0, 5000.0, 10000.0] {
        if v <= m {
            return m;
        }
    }
    v
}

fn watts(w: f64) -> String {
    if w >= 100.0 { format!("{w:.0} W") } else { format!("{w:.1} W") }
}

/// Relay value as a short label: 50n, 2.2µ, 470p, 1n
fn short(v: i64, unit_small: &str, unit_big: &str) -> String {
    if v < 1000 {
        format!("{v}{unit_small}")
    } else {
        let s = format!("{:.1}", v as f64 / 1000.0);
        format!("{}{unit_big}", s.trim_end_matches(".0"))
    }
}

/// Narrower than this, the boxes stack one above the other (a phone)
const NARROW: u16 = 76;

pub fn draw(f: &mut Frame, t: &Tuner, app: &App) -> Hits {
    let mut hits = Hits::default();
    let narrow = f.area().width < NARROW;
    let key_rows = if narrow { 2 } else { 1 };
    let [header, body, keys] = Layout::vertical([Constraint::Length(1), Constraint::Min(10), Constraint::Length(key_rows)]).areas(f.area());
    draw_header(f, header, t, app, &mut hits);
    if narrow {
        let [power, swr, tuner, relays, events] = Layout::vertical([
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(13),
            Constraint::Length(6),
            Constraint::Min(3),
        ])
        .areas(body);
        draw_power(f, power, t, app);
        draw_swr(f, swr, t, app);
        draw_tuner(f, tuner, t, app, &mut hits);
        draw_relays(f, relays, t, &mut hits);
        draw_events(f, events, t);
    } else {
        let top_h = (body.height / 2).max(9);
        let [top, bottom] = Layout::vertical([Constraint::Length(top_h), Constraint::Min(8)]).areas(body);
        let [power, swr] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(top);
        draw_power(f, power, t, app);
        draw_swr(f, swr, t, app);
        let tuner_w = 36.min(bottom.width / 2);
        let [tuner, right] = Layout::horizontal([Constraint::Length(tuner_w), Constraint::Min(30)]).areas(bottom);
        draw_tuner(f, tuner, t, app, &mut hits);
        let [relays, events] = Layout::vertical([Constraint::Length(6), Constraint::Min(3)]).areas(right);
        draw_relays(f, relays, t, &mut hits);
        draw_events(f, events, t);
    }
    if app.winding.is_some() {
        hits = Hits::default();
        draw_winding(f, body, t, app, &mut hits);
        let keys_list = [
            ("Esc", "cancel", Action::WindingCancel, false),
            ("↑↓", "choose", Action::WindingCancel, false),
            ("Space", "tunes from too", Action::WindingSetMin, false),
            ("Enter", "apply", Action::WindingApply, false),
        ];
        draw_key_list(f, keys, &keys_list, &mut hits);
    } else if app.connect.is_some() {
        hits = Hits::default(); // the box takes all the clicks while it is open
        draw_connect(f, body, app, &mut hits);
        let keys_list = [
            ("Esc", if app.port.is_empty() { "quit" } else { "close" }, Action::CloseConnect, false),
            ("↑↓", "choose", Action::ConnectRefresh, false),
            ("Enter", "connect", Action::ConnectGo, false),
            ("Tab", "address", Action::ConnectType, false),
            ("F5", "search again", Action::ConnectRefresh, false),
        ];
        draw_key_list(f, keys, &keys_list, &mut hits);
        hits.areas.retain(|(r, a)| r.y != keys.y || *a != Action::ConnectRefresh || r.x > keys.x + 20); // ↑↓ is not a button
    } else if app.settings.is_some() {
        hits = Hits::default(); // the box takes all the clicks while it is open
        draw_settings(f, body, app, &mut hits);
        let typing = app.esp.present == Some(true) && app.esp.may_set_wifi;
        let keys_list = [
            ("Esc", "close", Action::CloseSettings, false),
            ("Tab", "field", Action::Focus(Field::Network), false),
            ("↑↓", "network", Action::PickNet(0), false),
            ("Enter", "connect", Action::Connect, false),
            ("F5", "scan", Action::Scan, false),
        ];
        draw_key_list(f, keys, &keys_list[..if typing { 5 } else { 1 }], &mut hits);
    } else {
        draw_keys(f, keys, t, app, &mut hits);
    }
    hits
}

fn draw_header(f: &mut Frame, area: Rect, t: &Tuner, app: &App, hits: &mut Hits) {
    let live = t.last_rx.is_some_and(|r| r.elapsed().as_secs() < 12);
    let (dot, state) = match (&app.link_error, live) {
        (Some(e), _) => (Span::styled("● ", Style::new().fg(RED)), Span::styled(e.clone(), Style::new().fg(RED))),
        (None, true) => (Span::styled("● ", Style::new().fg(GREEN)), Span::styled("live", Style::new().fg(MAIN_FG))),
        (None, false) => (Span::styled("● ", Style::new().fg(YELLOW)), Span::styled("waiting for the tuner", Style::new().fg(MAIN_FG))),
    };
    let mut left = vec![
        Span::styled(" atu", Style::new().fg(TITLE).add_modifier(Modifier::BOLD)),
        Span::styled("top", Style::new().fg(HI).add_modifier(Modifier::BOLD)),
        Span::styled("  ", Style::new()),
    ];
    if area.width >= NARROW {
        left.push(Span::styled(app.port.clone(), Style::new().fg(GRAPH_TEXT))); // a phone has no room for it
        left.push(Span::raw("  "));
    }
    left.push(dot);
    left.push(state);
    // the ESP32 bridge's web page: click to open it in the browser
    if app.esp.present == Some(true) && !app.in_browser {
        let start = Line::from(left.clone()).width() as u16 + 2;
        let urls = app.esp.web_urls();
        let short = |u: &str| u.trim_start_matches("http://").trim_end_matches('/').to_string();
        let web = match (&app.web_reach, urls.first()) {
            (_, None) => vec![Span::raw("  "), Span::styled("○ ", Style::new().fg(INACTIVE)), Span::styled("web server offline", Style::new().fg(GRAPH_TEXT))],
            (Some(None), Some(first)) => vec![
                Span::raw("  "),
                Span::styled("● ", Style::new().fg(YELLOW)),
                Span::styled("web server up, not reachable from here ", Style::new().fg(MAIN_FG)),
                Span::styled(short(first), Style::new().fg(GRAPH_TEXT)),
            ],
            (reach, Some(first)) => {
                let url = reach.clone().flatten().unwrap_or_else(|| first.clone());
                vec![
                    Span::raw("  "),
                    Span::styled("● ", Style::new().fg(GREEN)),
                    Span::styled("web server online ", Style::new().fg(MAIN_FG)),
                    Span::styled(short(&url), Style::new().fg(TITLE).add_modifier(Modifier::UNDERLINED)),
                ]
            }
        };
        let width = Line::from(web.clone()).width() as u16;
        if !urls.is_empty() {
            hits.areas.push((Rect { x: area.x + start, y: area.y, width: width.saturating_sub(2), height: 1 }, Action::OpenWeb));
        }
        left.extend(web);
    } else if let Some(b) = &t.build {
        left.push(Span::styled(format!("  ATU-100 · {b}"), Style::new().fg(GRAPH_TEXT)));
    }
    f.render_widget(Paragraph::new(Line::from(left)), area);
    let time = Line::from(Span::styled(format!("{} ", crate::tuner::now()), Style::new().fg(TITLE))).right_aligned();
    f.render_widget(Paragraph::new(time), area);
}

fn draw_power(f: &mut Frame, area: Rect, t: &Tuner, app: &App) {
    let alarm_on = t.high_power && app.started.elapsed().as_millis() / 500 % 2 == 0;
    let mut block = boxed("¹", "power", if alarm_on { RED } else { BOX_POWER });
    if t.high_power && !t.tuning {
        block = block.title(Line::from(Span::styled(" HIGH POWER ", Style::new().fg(TITLE).bg(HI).add_modifier(Modifier::BOLD))).centered());
    }
    if t.tuning {
        let on = app.started.elapsed().as_millis() / 500 % 2 == 0;
        let style = if on { Style::new().fg(TITLE).bg(HI).add_modifier(Modifier::BOLD) } else { Style::new().fg(HI).add_modifier(Modifier::BOLD) };
        block = block.title(Line::from(Span::styled(" TUNING · transmit a carrier ", style)).centered());
    }
    // fixed by the detector winding when known, else following the peak
    let peak = t.power_hist.iter().copied().fold(t.power, f64::max);
    let scale = t.power_scale().unwrap_or_else(|| nice_max((peak * 1.2).max(1.0)));
    block = block.title(corner(format!("0 – {}", watts(scale)), BOX_POWER));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 3 {
        return;
    }
    let mut line = vec![Span::styled(format!(" {}", watts(t.power)), Style::new().fg(TITLE).add_modifier(Modifier::BOLD))];
    let mut right = vec![];
    if let (Some(a), Some(e)) = (t.antenna, t.efficiency) {
        right.push(Span::styled("antenna ", Style::new().fg(GRAPH_TEXT)));
        right.push(Span::styled(watts(a), Style::new().fg(MAIN_FG)));
        right.push(Span::styled(format!("  {e}% "), Style::new().fg(gradient(1.0 - e as f64 / 100.0))));
    }
    let used = Line::from(line.clone()).width() + Line::from(right.clone()).width();
    if used + 14 <= inner.width as usize {
        line.push(Span::styled("  transmitter", Style::new().fg(GRAPH_TEXT)));
    }
    let row = Rect { height: 1, ..inner };
    f.render_widget(Paragraph::new(Line::from(line)), row);
    f.render_widget(Paragraph::new(Line::from(right).right_aligned()), row);
    let bar = Rect { x: inner.x + 1, y: inner.y + 1, width: inner.width.saturating_sub(2), height: 1 };
    meter(f.buffer_mut(), bar, t.power / scale);
    // the HighPower limit as a mark on the meter
    if let Some(a) = t.power_alarm().filter(|a| *a < scale) {
        let x = bar.x + ((a / scale) * bar.width.saturating_sub(1) as f64).round() as u16;
        if let Some(cell) = f.buffer_mut().cell_mut((x, bar.y)) {
            cell.set_symbol("┃").set_fg(TITLE);
        }
    }
    let g = Rect { x: inner.x, y: inner.y + 2, width: inner.width, height: inner.height - 2 };
    graph(f.buffer_mut(), g, t.power_hist.iter().map(|p| p / scale));
}

fn draw_swr(f: &mut Frame, area: Rect, t: &Tuner, app: &App) {
    let alarm_on = t.high_swr && app.started.elapsed().as_millis() / 500 % 2 == 0;
    let border = if alarm_on { RED } else { BOX_SWR };
    let mut block = boxed("²", "swr", border);
    block = block.title(corner("1.0 – 3.0".into(), border));
    if t.high_swr {
        block = block.title(Line::from(Span::styled(" HIGH SWR ", Style::new().fg(TITLE).bg(HI).add_modifier(Modifier::BOLD))).centered());
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 3 {
        return;
    }
    let value = if t.swr == 0.0 { " -.--".to_string() } else { format!(" {:.2}", t.swr) };
    let mut line = vec![Span::styled(value, Style::new().fg(swr_color(t.swr)).add_modifier(Modifier::BOLD))];
    // N7DDC's firmware measures the SWR only from 1 W up
    let note = match (t.swr == 0.0, t.power > 0.0) {
        (true, true) => "  under 1 W: not measured",
        (true, false) => "  no power",
        _ => "",
    };
    line.push(Span::styled(note, Style::new().fg(GRAPH_TEXT)));
    let row = Rect { height: 1, ..inner };
    f.render_widget(Paragraph::new(Line::from(line)), row);
    let limit = match t.alarm() {
        Some(a) => format!("alarm {a:.1} "),
        None => "alarm off ".into(),
    };
    f.render_widget(Paragraph::new(Line::from(Span::styled(limit, Style::new().fg(GRAPH_TEXT))).right_aligned()), row);
    let bar = Rect { x: inner.x + 1, y: inner.y + 1, width: inner.width.saturating_sub(2), height: 1 };
    let frac = if t.swr == 0.0 { 0.0 } else { (t.swr - 1.0) / 2.0 };
    meter(f.buffer_mut(), bar, frac);
    // the alarm limit as a mark on the meter
    if let Some(a) = t.alarm().filter(|a| *a <= 3.0) {
        let x = bar.x + (((a - 1.0) / 2.0) * bar.width.saturating_sub(1) as f64).round() as u16;
        if let Some(cell) = f.buffer_mut().cell_mut((x, bar.y)) {
            cell.set_symbol("┃").set_fg(TITLE);
        }
    }
    let g = Rect { x: inner.x, y: inner.y + 2, width: inner.width, height: inner.height - 2 };
    graph(f.buffer_mut(), g, t.swr_hist.iter().map(|s| if *s == 0.0 { 0.0 } else { (s - 1.0) / 2.0 }));
}

fn label(text: &str, width: usize) -> Span<'static> {
    Span::styled(format!(" {text:<width$}"), Style::new().fg(GRAPH_TEXT))
}

fn switch(on: bool) -> Vec<Span<'static>> {
    if on {
        vec![Span::styled("● ", Style::new().fg(GREEN)), Span::styled("on ", Style::new().fg(TITLE).add_modifier(Modifier::BOLD))]
    } else {
        vec![Span::styled("○ ", Style::new().fg(INACTIVE)), Span::styled("off", Style::new().fg(MAIN_FG))]
    }
}

fn draw_tuner(f: &mut Frame, area: Rect, t: &Tuner, app: &App, hits: &mut Hits) {
    let block = boxed("³", "tuner", BOX_TUNER);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let value = |s: String| Span::styled(s, Style::new().fg(TITLE));
    let dim = |s: &str| Span::styled(s.to_string(), Style::new().fg(GRAPH_TEXT));
    let w = 12;
    let mut lines: Vec<(Line, Option<Action>)> = vec![];
    lines.push((Line::from(vec![label("Inductance", w), value(format!("{} nH", t.ind_nh))]), None));
    lines.push((Line::from(vec![label("Capacitance", w), value(format!("{} pF", t.cap_pf))]), None));
    lines.push((
        Line::from(vec![label("Order", w), value(if t.order_lc { "LC".into() } else { "CL".into() }), dim(if t.order_lc { "  C at antenna" } else { "  C at radio" })]),
        None,
    ));
    let det = match (app.detector, t.fwd_mv, t.rev_mv) {
        (true, Some(fw), Some(rv)) => Line::from(vec![label("Detector", w), value(format!("{fw}/{rv}")), dim(" mV fwd/ref")]),
        (true, _, _) => Line::from(vec![label("Detector", w), dim("reading…")]),
        (false, _, _) => Line::from(vec![label("Detector", w), dim("press "), Span::styled("v", Style::new().fg(HI)), dim(" to watch")]),
    };
    lines.push((det, Some(Action::Detector)));
    lines.push((Line::from(Span::styled(" ".to_string() + &"─".repeat(inner.width.saturating_sub(2) as usize), Style::new().fg(DIV))), None));
    let mut auto = vec![label("Auto", w)];
    auto.extend(switch(t.auto));
    if let Some(a) = t.auto_swr() {
        auto.push(dim(&format!("  above {a:.1}")));
    }
    lines.push((Line::from(auto), Some(Action::Auto)));
    let mut bypass = vec![label("Bypass", w)];
    bypass.extend(switch(t.bypass));
    lines.push((Line::from(bypass), Some(Action::Bypass)));
    let search = match t.search_original() {
        Some(true) => value("N7DDC original".into()),
        Some(false) => value("log grid".into()),
        None => dim("…"),
    };
    lines.push((Line::from(vec![label("Search", w), search, dim("  g")]), Some(Action::Search)));
    let alarm = match t.alarm() {
        Some(a) => value(format!("SWR {a:.1}")),
        None => value("off".into()),
    };
    let alarm_w = Line::from(alarm.clone()).width() as u16;
    lines.push((Line::from(vec![label("HighSWR", w), alarm, Span::raw("  "), Span::styled(" − ", Style::new().fg(TITLE).bg(DIV)), Span::raw(" "), Span::styled(" + ", Style::new().fg(TITLE).bg(DIV))]), None));
    match t.min_power() {
        Some(p) => lines.push((Line::from(vec![label("Tunes from", w), value(format!("{p} W")), dim("  click: 1 W / 5 W")]), Some(Action::MinToggle))),
        None => lines.push((Line::from(vec![label("Tunes from", w), dim("…")]), None)),
    }
    let winding = match (t.winding(), t.power_alarm()) {
        (Some(k), Some(a)) => vec![label("Winding", w), value(format!("1:{k}")), dim(&format!("  to {a:.0} W  click"))],
        (Some(k), None) => vec![label("Winding", w), value(format!("1:{k}")), dim("  high power mode")],
        _ => vec![label("Winding", w), dim("…")],
    };
    lines.push((Line::from(winding), Some(Action::WindingOpen)));
    for (i, (line, action)) in lines.into_iter().enumerate() {
        if i as u16 >= inner.height {
            break;
        }
        let row = Rect { y: inner.y + i as u16, height: 1, ..inner };
        if let Some(a) = action {
            hits.areas.push((row, a));
        }
        if i == 8 {
            hits.alarm_row = Some(row);
            // the − and + buttons after the value
            let x = row.x + 1 + w as u16 + alarm_w + 2;
            hits.areas.push((Rect { x, y: row.y, width: 3, height: 1 }, Action::AlarmDown));
            hits.areas.push((Rect { x: x + 4, y: row.y, width: 3, height: 1 }, Action::AlarmUp));
        }
        f.render_widget(Paragraph::new(line), row);
    }
}

fn draw_relays(f: &mut Frame, area: Rect, t: &Tuner, hits: &mut Hits) {
    let mut block = boxed("⁴", "relays", BOX_RELAYS);
    if !t.values_from_tuner {
        block = block.title(corner("default values".into(), BOX_RELAYS));
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 3 {
        return;
    }
    let chip_w = ((inner.width.saturating_sub(4 + 6)) / 7).clamp(4, 12);
    let buf = f.buffer_mut();
    let rows: [(&str, u8, [i64; 7], &str, &str, fn(u8) -> Action); 2] = [
        ("L", t.ind_bits, t.ind_values, "n", "µ", Action::Ind),
        ("C", t.cap_bits, t.cap_values, "p", "n", Action::Cap),
    ];
    for (r, (name, bits, values, small, big, act)) in rows.iter().enumerate() {
        let y = inner.y + r as u16;
        buf.set_string(inner.x + 1, y, *name, Style::new().fg(TITLE).add_modifier(Modifier::BOLD));
        for i in 0..7u16 {
            let x = inner.x + 3 + i * (chip_w + 1);
            if x + chip_w > inner.x + inner.width {
                break;
            }
            let on = bits >> i & 1 == 1;
            let text = short(values[i as usize], small, big);
            let text = format!("{text:^w$}", w = chip_w as usize);
            let style = if on { Style::new().fg(TITLE).bg(ON_BG).add_modifier(Modifier::BOLD) } else { Style::new().fg(GRAPH_TEXT).bg(DIV) };
            buf.set_string(x, y, &text, style);
            hits.areas.push((Rect { x, y, width: chip_w, height: 1 }, act(i as u8)));
        }
    }
    let y = inner.y + 2;
    let side = if t.order_lc { " LC  C at antenna " } else { " CL  C at radio " };
    buf.set_string(inner.x + 3, y, side, Style::new().fg(TITLE).bg(DIV));
    hits.areas.push((Rect { x: inner.x + 3, y, width: side.chars().count() as u16, height: 1 }, Action::Side));
    let total = format!("{} nH · {} pF ", t.ind_nh, t.cap_pf);
    let x = (inner.x + inner.width).saturating_sub(total.chars().count() as u16);
    buf.set_string(x, y, total, Style::new().fg(MAIN_FG));
    if inner.height > 3 {
        buf.set_string(inner.x + 1, inner.y + 3, "click a relay to switch it", Style::new().fg(INACTIVE));
    }
}

fn draw_events(f: &mut Frame, area: Rect, t: &Tuner) {
    let block = boxed("⁵", "events", BOX_EVENTS);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let lines: Vec<Line> = t
        .events
        .iter()
        .take(inner.height as usize)
        .map(|e| {
            let color = match e.kind {
                Kind::Info => MAIN_FG,
                Kind::Good => GREEN,
                Kind::Warn => YELLOW,
                Kind::Bad => RED,
            };
            Line::from(vec![Span::styled(format!(" {} ", e.time), Style::new().fg(GRAPH_TEXT)), Span::styled(e.text.clone(), Style::new().fg(color))])
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_keys(f: &mut Frame, area: Rect, t: &Tuner, app: &App, hits: &mut Hits) {
    let mut keys = vec![
        ("t", "tune", Action::Tune, t.tuning),
        ("b", "bypass", Action::Bypass, t.bypass),
        ("a", "auto", Action::Auto, t.auto),
        ("r", "reset", Action::Reset, false),
        ("s", "status", Action::Status, false),
        ("v", "detector", Action::Detector, app.detector),
        ("g", "search", Action::Search, false),
        ("o", "settings", Action::Settings, false),
        ("q", "quit", Action::Quit, false),
    ];
    if app.in_browser {
        keys.retain(|k| k.2 != Action::Quit);
    }
    // On a narrow terminal leave out the buttons least needed (their keys
    // still work) so settings and quit always show
    for drop in [Action::Status, Action::Search, Action::Detector, Action::Reset] {
        if !keys_fit(&keys, area) {
            keys.retain(|k| k.2 != drop);
        }
    }
    draw_key_list(f, area, &keys, hits);
}

/// Do the buttons fit on the rows, wrapped as draw_key_list does?
fn keys_fit_gap(keys: &[(&str, &str, Action, bool)], area: Rect, gap: u16) -> bool {
    let (mut x, mut rows) = (gap, 1u16);
    for (k, n, _, _) in keys {
        let w = (k.chars().count() + n.len() + 3) as u16;
        if x + w > area.width {
            rows += 1;
            x = gap;
        }
        x += w + gap;
    }
    rows <= area.height
}

fn keys_fit(keys: &[(&str, &str, Action, bool)], area: Rect) -> bool {
    keys_fit_gap(keys, area, 0)
}

fn draw_key_list(f: &mut Frame, area: Rect, keys: &[(&str, &str, Action, bool)], hits: &mut Hits) {
    // a gap between buttons when there is room for it
    let gap = if keys_fit_gap(keys, area, 1) { 1 } else { 0 };
    let mut x = area.x + gap;
    let mut y = area.y;
    for &(key, name, action, active) in keys {
        let width = (key.chars().count() + name.len() + 3) as u16;
        if x + width > area.x + area.width {
            // on to the next row, if there is one
            if y + 1 >= area.y + area.height {
                break;
            }
            y += 1;
            x = area.x + gap;
        }
        let area = Rect { y, ..area };
        let bg = if active { ON_BG } else { DIV };
        let buf = f.buffer_mut();
        buf.set_string(x, area.y, " ", Style::new().bg(bg));
        buf.set_string(x + 1, area.y, key, Style::new().fg(HI).bg(bg).add_modifier(Modifier::BOLD));
        buf.set_string(x + 1 + key.chars().count() as u16, area.y, format!(" {name} "), Style::new().fg(MAIN_FG).bg(bg));
        hits.areas.push((Rect { x, y: area.y, width, height: 1 }, action));
        x += width + gap;
    }
}

/// A clickable button in the settings box; returns its width
fn button(buf: &mut Buffer, x: u16, y: u16, text: &str, action: Action, on: bool, hits: &mut Hits) -> u16 {
    let text = format!(" {text} ");
    let w = text.chars().count() as u16;
    let style = if on { Style::new().fg(TITLE).bg(ON_BG).add_modifier(Modifier::BOLD) } else { Style::new().fg(INACTIVE).bg(DIV) };
    buf.set_string(x, y, &text, style);
    if on {
        hits.areas.push((Rect { x, y, width: w, height: 1 }, action));
    }
    w
}

/// The settings box, over the middle of the screen: the link, and the ESP32
/// bridge's Wi-Fi when there is one
fn draw_settings(f: &mut Frame, body: Rect, app: &App, hits: &mut Hits) {
    let Some(set) = &app.settings else { return };
    let esp = &app.esp;
    let usb = esp.present == Some(true) && esp.may_set_wifi;
    let nets = if usb { esp.networks.len().min(6) as u16 } else { 0 };
    let want_h = match esp.present {
        Some(true) if usb => 17 + nets.max(1) + if set.fixed { 3 } else { 0 } + (body.width < 64) as u16,
        Some(true) => 13,
        _ => 10,
    };
    // Too short for everything (a phone with its keyboard up): keep the
    // fields and the buttons, drop the status rows and shorten the list
    let compact = usb && body.height < want_h;
    let w = body.width.saturating_sub(2).min(74);
    let h = body.height.min(want_h);
    let area = Rect { x: body.x + (body.width - w) / 2, y: body.y + (body.height - h) / 2, width: w, height: h };
    f.render_widget(Clear, area);
    let block = boxed("⁶", "settings", BOX_SETTINGS).title(corner("Esc to close ✕".into(), BOX_SETTINGS));
    let inner = block.inner(area);
    // a tap on "Esc to close": no keyboard on a phone
    hits.areas.push((Rect { x: area.x + area.width / 2, y: area.y, width: area.width - area.width / 2, height: 1 }, Action::CloseSettings));
    f.render_widget(block, area);
    let buf = f.buffer_mut();
    let bottom = inner.y + inner.height;
    let mut y = inner.y;
    let lw = 12usize;
    let put = |buf: &mut Buffer, y: u16, spans: Vec<Span>| {
        if y < bottom {
            buf.set_line(inner.x, y, &Line::from(spans), inner.width);
        }
    };
    let value = |s: String| Span::styled(s, Style::new().fg(TITLE));
    let dim = |s: String| Span::styled(s, Style::new().fg(GRAPH_TEXT));
    let colored = |s: String, c: Color| Span::styled(s, Style::new().fg(c));
    let rule = |buf: &mut Buffer, y: u16| {
        if y < bottom {
            buf.set_string(inner.x + 1, y, "─".repeat(inner.width.saturating_sub(2) as usize), Style::new().fg(DIV));
        }
    };

    let how = match (esp.present, esp.over_usb, app.port.starts_with("socket://")) {
        (Some(true), true, _) => "USB to the ESP32 bridge",
        (Some(true), false, _) if esp.may_set_wifi => "the ESP32 bridge's own hotspot",
        (Some(true), false, _) => "Wi-Fi to the ESP32 bridge",
        (None, _, _) => "looking for an ESP32 bridge…",
        (Some(false), _, true) => "network, no ESP32 bridge",
        (Some(false), _, false) => "straight to the tuner, or through the Mega",
    };
    if !compact {
        put(buf, y, vec![label("Link", lw), value(app.port.clone()), dim(format!("  {how}"))]);
        if !app.in_browser {
            let bw = " change ".chars().count() as u16;
            button(buf, (inner.x + inner.width).saturating_sub(bw + 1), y, "change", Action::ChangeLink, true, hits);
        }
        y += 1;
        if esp.present == Some(true) {
            put(buf, y, vec![label("ESP32", lw), value(format!("bridge {}", esp.version))]);
            y += 1;
        }
        rule(buf, y);
        y += 1;
    }

    match esp.present {
        None => {
            put(buf, y, vec![dim(" Asking the link for an ESP32 bridge…".into())]);
            return;
        }
        Some(false) => {
            for text in [
                " No ESP32 bridge answered on this link.",
                "",
                " The Wi-Fi settings are for the ESP32 bridge. With the tuner",
                " connected straight to the computer, or through the Mega,",
                " there is nothing to set here; everything else works as usual.",
            ] {
                put(buf, y, vec![dim(text.into())]);
                y += 1;
            }
            return;
        }
        Some(true) => {}
    }

    if !compact {
        // Wi-Fi state
        let state = match esp.wifi.as_str() {
            "Connected" => vec![colored("● ".into(), GREEN), value("connected  ".into()), value(esp.ssid.clone()), dim(format!("  {} {} dBm", bars(esp.rssi), esp.rssi))],
            "Connecting" => vec![colored("● ".into(), YELLOW), value(format!("joining {}…", esp.ssid))],
            "Failed" => vec![colored("● ".into(), RED), colored(format!("cannot join {}", esp.ssid), RED), dim("  trying again".into())],
            _ => vec![colored("○ ".into(), INACTIVE), value("not set up".into())],
        };
        let mut line = vec![label("Wi-Fi", lw)];
        line.extend(state);
        put(buf, y, line);
        y += 1;
        if let Some(port) = esp.wifi_port() {
            let addr = port.trim_start_matches("socket://").to_string();
            let how = if esp.static_ip.is_empty() { "from the router" } else { "fixed" };
            let host = if esp.host.is_empty() { format!("  {how}") } else { format!("  {how} · {}", esp.host) };
            put(buf, y, vec![label("Address", lw), value(addr), dim(host)]);
            if esp.over_usb && !app.in_browser && y < bottom {
                let bw = " use Wi-Fi ".chars().count() as u16;
                button(buf, (inner.x + inner.width).saturating_sub(bw + 1), y, "use Wi-Fi", Action::UseWifi, true, hits);
            }
            y += 1;
        }
        let mut line = vec![label("Hotspot", lw)];
        if esp.hotspot {
            line.extend([colored("● ".into(), GREEN), value(format!("on  {}", esp.hotspot_name)), dim(format!("  {}", esp.hotspot_ip))]);
        } else {
            line.extend([colored("○ ".into(), INACTIVE), value("off".into()), dim("  on when there is no network".into())]);
        }
        put(buf, y, line);
        if usb && y < bottom {
            let text = if esp.hotspot_always { "● always on" } else { "○ always on" };
            let bw = text.chars().count() as u16 + 2;
            if inner.width < 60 {
                // a phone: on a row of its own, under the hotspot
                y += 1;
                if y < bottom {
                    button(buf, inner.x + 1 + lw as u16, y, text, Action::HotspotAlways, true, hits);
                }
            } else {
                button(buf, (inner.x + inner.width).saturating_sub(bw + 1), y, text, Action::HotspotAlways, true, hits);
            }
        }
        y += 1;
        rule(buf, y);
        y += 1;
    
    }

    if !usb {
        let note = format!("Wi-Fi settings change over USB, or from the bridge's own hotspot ({}): connect that way to choose another network.", esp.hotspot_name);
        for (i, line) in wrap(&note, inner.width.saturating_sub(2) as usize).into_iter().enumerate() {
            put(buf, y + i as u16, vec![dim(format!(" {line}"))]);
        }
        return;
    }

    // Networks
    put(buf, y, vec![label("Networks", lw)]);
    if y < bottom {
        let x = inner.x + 1 + lw as u16;
        let bw = button(buf, x, y, "scan", Action::Scan, !esp.scanning, hits);
        let note = if esp.scanning {
            "  scanning…".to_string()
        } else if esp.networks.is_empty() {
            "  none found yet".to_string()
        } else {
            format!("  {} found", esp.networks.len())
        };
        buf.set_string(x + bw, y, note, Style::new().fg(GRAPH_TEXT));
    }
    y += 1;
    // rows the fields and buttons below need: leave them room
    let below = 7 + if set.fixed { 3 } else { 0 };
    let nets = nets.min(bottom.saturating_sub(y).saturating_sub(below));
    for (i, net) in esp.networks.iter().take(nets as usize).enumerate() {
        if y >= bottom {
            break;
        }
        let picked = set.selected == Some(i) || (set.selected.is_none() && set.ssid == net.ssid);
        let mark = if picked { "▸ " } else { "  " };
        let name: String = net.ssid.chars().take(28).collect();
        let style = if picked { Style::new().fg(TITLE).add_modifier(Modifier::BOLD) } else { Style::new().fg(MAIN_FG) };
        let row = Line::from(vec![
            Span::raw(" ".repeat(lw + 1)),
            Span::styled(mark, Style::new().fg(HI)),
            Span::styled(format!("{name:<29}"), style),
            Span::styled(format!("{} {:>4} dBm", bars(net.rssi), net.rssi), Style::new().fg(gradient(((-net.rssi - 45) as f64 / 45.0).clamp(0.0, 1.0)))),
            Span::styled(if net.secure { "  secured" } else { "  open" }, Style::new().fg(GRAPH_TEXT)),
        ]);
        buf.set_line(inner.x, y, &row, inner.width);
        hits.areas.push((Rect { x: inner.x, y, width: inner.width, height: 1 }, Action::PickNet(i as u8)));
        y += 1;
    }
    if esp.networks.is_empty() {
        y += 1;
    }
    y += 1;

    // The fields: network, password, and the address
    let field_w = 32u16.min(inner.width.saturating_sub(lw as u16 + 10)); // narrower on a phone, "show" still beside it
    let x0 = inner.x + 1 + lw as u16;
    let field_row = |buf: &mut Buffer, y: u16, name: &str, text: String, field: Field, hits: &mut Hits| {
        if y >= bottom {
            return;
        }
        let focused = set.field == field;
        buf.set_line(inner.x, y, &Line::from(label(name, lw)), inner.width);
        let shown: String = text.chars().rev().take(field_w as usize - 1).collect::<Vec<_>>().into_iter().rev().collect();
        let bg = if focused { DIV } else { FIELD_BG };
        buf.set_string(x0, y, format!(" {shown:<w$}", w = field_w as usize - 1), Style::new().fg(TITLE).bg(bg));
        if focused {
            let cx = x0 + 1 + shown.chars().count() as u16;
            buf.set_string(cx, y, "▏", Style::new().fg(HI).bg(bg).add_modifier(Modifier::BOLD));
        }
        hits.areas.push((Rect { x: x0, y, width: field_w, height: 1 }, Action::Focus(field)));
    };
    field_row(buf, y, "Network", set.ssid.clone(), Field::Network, hits);
    y += 1;
    let masked = if set.show_pass { set.pass.clone() } else { "•".repeat(set.pass.chars().count()) };
    field_row(buf, y, "Password", masked, Field::Password, hits);
    if y < bottom {
        let bw = button(buf, x0 + field_w + 2, y, if set.show_pass { "hide" } else { "show" }, Action::ShowPass, true, hits);
        if set.pass.is_empty() && set.ssid == esp.ssid && !esp.ssid.is_empty() {
            buf.set_string(x0 + field_w + 3 + bw, y, "empty keeps the saved one", Style::new().fg(INACTIVE));
        }
    }
    y += 1;
    if y < bottom {
        buf.set_line(inner.x, y, &Line::from(label("IP address", lw)), inner.width);
        let pick = |buf: &mut Buffer, x: u16, text: &str, on: bool, action: Action, hits: &mut Hits| -> u16 {
            let text = format!(" {} {text} ", if on { "●" } else { "○" });
            let w = text.chars().count() as u16;
            let style = if on { Style::new().fg(TITLE).bg(ON_BG).add_modifier(Modifier::BOLD) } else { Style::new().fg(MAIN_FG).bg(DIV) };
            buf.set_string(x, y, &text, style);
            hits.areas.push((Rect { x, y, width: w, height: 1 }, action));
            w
        };
        let w1 = pick(buf, x0, "from the router", !set.fixed, Action::AddrFixed(false), hits);
        pick(buf, x0 + w1 + 1, "fixed", set.fixed, Action::AddrFixed(true), hits);
    }
    y += 1;
    if set.fixed {
        field_row(buf, y, "  Address", set.ip.clone(), Field::Ip, hits);
        y += 1;
        let gw = if set.gateway.is_empty() && set.field != Field::Gateway { String::new() } else { set.gateway.clone() };
        field_row(buf, y, "  Gateway", gw, Field::Gateway, hits);
        if set.gateway.is_empty() && y < bottom {
            buf.set_string(x0 + field_w + 2, y, "empty: x.x.x.1", Style::new().fg(INACTIVE));
        }
        y += 1;
        field_row(buf, y, "  Mask", set.mask.clone(), Field::Mask, hits);
        y += 1;
    }
    y += 1;
    if y < bottom {
        let x = inner.x + 1 + lw as u16;
        let cw = button(buf, x, y, "connect", Action::Connect, !set.ssid.is_empty(), hits);
        button(buf, x + cw + 2, y, "forget network", Action::Forget, !esp.ssid.is_empty() || esp.wifi != "Off", hits);
    }
    y += 1;
    if let Some((text, error)) = &esp.notice {
        put(buf, y, vec![Span::raw(" ".repeat(lw + 1)), colored(text.clone(), if *error { RED } else { GREEN })]);
    }
}

/// The connect screen: what is plugged in, what is on the network, what
/// was used before, an address to type, and the simulator
fn draw_connect(f: &mut Frame, body: Rect, app: &App, hits: &mut Hits) {
    use crate::connect::Kind as K;
    let Some(c) = &app.connect else { return };
    let groups: [(K, &str); 4] = [(K::Usb, "USB"), (K::Network, "Network"), (K::Recent, "Used before"), (K::Sim, "Try it")];
    let shown = groups.iter().filter(|(k, _)| c.entries.iter().any(|e| e.kind == *k) || *k == K::Network).count() as u16;
    let no_net = !c.entries.iter().any(|e| e.kind == K::Network) as u16;
    let want_h = c.entries.len() as u16 + shown * 2 + no_net + 4 + c.error.is_some() as u16;
    let w = body.width.saturating_sub(4).min(78);
    let h = body.height.min(want_h);
    let area = Rect { x: body.x + (body.width - w) / 2, y: body.y + (body.height - h) / 2, width: w, height: h };
    f.render_widget(Clear, area);
    let corner_text = if c.searching.is_some() { "searching the network…" } else { "F5 to search again" };
    let block = boxed("⁰", "connect", BOX_SETTINGS).title(corner(corner_text.into(), BOX_SETTINGS));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let buf = f.buffer_mut();
    let bottom = inner.y + inner.height;
    let mut y = inner.y;
    for (kind, title) in groups {
        let rows: Vec<(usize, &crate::connect::Entry)> = c.entries.iter().enumerate().filter(|(_, e)| e.kind == kind).collect();
        if rows.is_empty() && kind != K::Network {
            continue;
        }
        if y >= bottom {
            break;
        }
        buf.set_string(inner.x + 1, y, title, Style::new().fg(TITLE).add_modifier(Modifier::BOLD));
        y += 1;
        if rows.is_empty() && y < bottom {
            let note = if c.searching.is_some() { "   looking for ESP32 bridges…" } else { "   no ESP32 bridge answered (it needs Wi-Fi set up first, over USB)" };
            buf.set_string(inner.x + 1, y, note, Style::new().fg(GRAPH_TEXT));
            y += 1;
        }
        for (i, e) in rows {
            if y >= bottom {
                break;
            }
            let picked = i == c.selected && !c.typing;
            let mark = if picked { "▸ " } else { "  " };
            let style = if picked { Style::new().fg(TITLE).add_modifier(Modifier::BOLD) } else { Style::new().fg(MAIN_FG) };
            let port = match e.port.as_str() {
                crate::connect::SIM => "simulator".to_string(),
                crate::connect::SIM_LAN => "simulator + phone".to_string(),
                _ => e.port.clone(),
            };
            let mut line = vec![
                Span::raw("  "),
                Span::styled(mark, Style::new().fg(HI)),
                Span::styled(format!("{port:<24} "), style),
            ];
            if e.port == c.current {
                line.push(Span::styled("● connected  ", Style::new().fg(GREEN)));
            }
            line.push(Span::styled(e.what.clone(), Style::new().fg(GRAPH_TEXT)));
            let line = Line::from(line);
            buf.set_line(inner.x, y, &line, inner.width);
            hits.areas.push((Rect { x: inner.x, y, width: inner.width, height: 1 }, Action::ConnectPick(i as u8)));
            y += 1;
        }
        y += 1;
    }
    if y < bottom {
        buf.set_string(inner.x + 1, y, "Address", Style::new().fg(TITLE).add_modifier(Modifier::BOLD));
        let x = inner.x + 10;
        let fw = 34u16;
        let bg = if c.typing { DIV } else { FIELD_BG };
        let text = if c.address.is_empty() && !c.typing { "IP of the bridge, or a port".to_string() } else { c.address.clone() };
        let fg = if c.address.is_empty() && !c.typing { INACTIVE } else { TITLE };
        buf.set_string(x, y, format!(" {text:<w$}", w = fw as usize - 1), Style::new().fg(fg).bg(bg));
        if c.typing {
            buf.set_string(x + 1 + c.address.chars().count() as u16, y, "▏", Style::new().fg(HI).bg(bg).add_modifier(Modifier::BOLD));
        }
        hits.areas.push((Rect { x, y, width: fw, height: 1 }, Action::ConnectType));
        button(buf, x + fw + 2, y, "connect", Action::ConnectGo, true, hits);
        y += 1;
    }
    if let Some(e) = &c.error {
        if y < bottom {
            buf.set_string(inner.x + 1, y, e, Style::new().fg(RED));
        }
    }
}

/// The detector winding: a choice that must match the hardware, so it is
/// asked for in a box that says what goes wrong otherwise
fn draw_winding(f: &mut Frame, body: Rect, t: &Tuner, app: &App, hits: &mut Hits) {
    let Some(d) = &app.winding else { return };
    let w = body.width.saturating_sub(2).min(76);
    let h = body.height.min(if w < 70 { 27 } else { 19 });
    let area = Rect { x: body.x + (body.width - w) / 2, y: body.y + (body.height - h) / 2, width: w, height: h };
    f.render_widget(Clear, area);
    let block = boxed("⁷", "detector winding", RED).title(corner("Esc to cancel ✕".into(), RED));
    let inner = block.inner(area);
    hits.areas.push((Rect { x: area.x + area.width / 2, y: area.y, width: area.width - area.width / 2, height: 1 }, Action::WindingCancel));
    f.render_widget(block, area);
    let buf = f.buffer_mut();
    let bottom = inner.y + inner.height;
    let mut y = inner.y;
    let tw = inner.width.saturating_sub(2) as usize;
    let text = |buf: &mut Buffer, y: &mut u16, s: &str, c: Color| {
        for line in wrap(s, tw) {
            if *y < bottom {
                buf.set_string(inner.x + 1, *y, line, Style::new().fg(c));
            }
            *y += 1;
        }
    };
    text(buf, &mut y, "How is the tandem match (the SWR detector's transformer) wound?", TITLE);
    let now = t.winding().map(|k| format!("The tuner is set for 1:{k} now.")).unwrap_or_default();
    text(buf, &mut y, &now, GRAPH_TEXT);
    y += 1;
    let choices: [(u8, &str, &str); 2] = [
        (10, "1:10  stock", "5 to 100 W, alarm above 100 W, tunes from 5 W"),
        (5, "1:5   low power", "1 to 40 W, alarm above 35 W, tunes from 1 W"),
    ];
    let side = tw >= 66; // room for the description beside the choice
    for (k, name, what) in choices {
        if y >= bottom {
            break;
        }
        let on = d.choice == k;
        let style = if on { Style::new().fg(TITLE).bg(ON_BG).add_modifier(Modifier::BOLD) } else { Style::new().fg(MAIN_FG).bg(DIV) };
        let chip = format!(" {} {name:<16}", if on { "●" } else { "○" });
        buf.set_string(inner.x + 2, y, &chip, style);
        let rows = if side { 1 } else { 2 };
        hits.areas.push((Rect { x: inner.x, y, width: inner.width, height: rows }, Action::WindingChoose(k)));
        if side {
            buf.set_string(inner.x + 3 + chip.chars().count() as u16, y, what, Style::new().fg(GRAPH_TEXT));
        } else if y + 1 < bottom {
            buf.set_stringn(inner.x + 4, y + 1, what, tw.saturating_sub(3), Style::new().fg(GRAPH_TEXT));
        }
        y += rows;
    }
    y += 1;
    text(buf, &mut y, "This must match how the transformer is really wound. If it does not, every power and SWR reading is wrong, and so is the tuning.", YELLOW);
    if d.choice == 5 {
        y += 1;
        text(buf, &mut y, "With the 1:5 winding the detector overloads above about 40 W: keep the transmitter at 35 W or less. The board takes 100 W only with the stock 1:10 winding.", RED);
    }
    y += 1;
    if y < bottom {
        let min = if d.choice == 5 { 1 } else { 5 };
        let line = format!(" [{}] also set \"Tunes from\" to {min} W", if d.set_min { "x" } else { " " });
        buf.set_string(inner.x + 1, y, &line, Style::new().fg(MAIN_FG));
        hits.areas.push((Rect { x: inner.x, y, width: line.chars().count() as u16 + 1, height: 1 }, Action::WindingSetMin));
        y += 2;
    }
    if y < bottom {
        let x = inner.x + 2;
        let aw = button(buf, x, y, "apply", Action::WindingApply, true, hits);
        button(buf, x + aw + 2, y, "cancel", Action::WindingCancel, true, hits);
    }
}

/// Words into lines of at most `width` characters
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![];
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}
