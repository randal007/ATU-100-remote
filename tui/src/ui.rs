//! Drawing, in the style of btop: rounded boxes, gradient meters and braille
//! graphs, with btop's default theme colours.

use crate::tuner::{Kind, Tuner};
use crate::App;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
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

pub fn draw(f: &mut Frame, t: &Tuner, app: &App) -> Hits {
    let mut hits = Hits::default();
    let [header, body, keys] = Layout::vertical([Constraint::Length(1), Constraint::Min(10), Constraint::Length(1)]).areas(f.area());
    draw_header(f, header, t, app);
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
    draw_keys(f, keys, t, app, &mut hits);
    hits
}

fn draw_header(f: &mut Frame, area: Rect, t: &Tuner, app: &App) {
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
        Span::styled(app.port.clone(), Style::new().fg(GRAPH_TEXT)),
        Span::raw("  "),
        dot,
        state,
    ];
    if let Some(b) = &t.build {
        left.push(Span::styled(format!("  ATU-100 · {b}"), Style::new().fg(GRAPH_TEXT)));
    }
    f.render_widget(Paragraph::new(Line::from(left)), area);
    let time = Line::from(Span::styled(format!("{} ", crate::tuner::now()), Style::new().fg(TITLE))).right_aligned();
    f.render_widget(Paragraph::new(time), area);
}

fn draw_power(f: &mut Frame, area: Rect, t: &Tuner, app: &App) {
    let mut block = boxed("¹", "power", BOX_POWER);
    if t.tuning {
        let on = app.started.elapsed().as_millis() / 500 % 2 == 0;
        let style = if on { Style::new().fg(TITLE).bg(HI).add_modifier(Modifier::BOLD) } else { Style::new().fg(HI).add_modifier(Modifier::BOLD) };
        block = block.title(Line::from(Span::styled(" TUNING · transmit a carrier ", style)).centered());
    }
    let peak = t.power_hist.iter().copied().fold(t.power, f64::max);
    let scale = nice_max((peak * 1.2).max(1.0)); // some headroom above the peak
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
    line.push(Span::styled(if t.swr == 0.0 { "  no power" } else { "" }, Style::new().fg(GRAPH_TEXT)));
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
    lines.push((Line::from(vec![label("HighSWR", w), alarm, dim("  [ ] or wheel")]), None));
    if let Some(p) = t.min_power() {
        lines.push((Line::from(vec![label("Tunes from", w), value(format!("{p} W"))]), None));
    }
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
    let keys: [(&str, &str, Action, bool); 8] = [
        ("t", "tune", Action::Tune, t.tuning),
        ("b", "bypass", Action::Bypass, t.bypass),
        ("a", "auto", Action::Auto, t.auto),
        ("r", "reset", Action::Reset, false),
        ("s", "status", Action::Status, false),
        ("v", "detector", Action::Detector, app.detector),
        ("g", "search", Action::Search, false),
        ("q", "quit", Action::Quit, false),
    ];
    // a gap between buttons when there is room for it
    let total: usize = keys.iter().map(|(k, n, _, _)| k.len() + n.len() + 3).sum();
    let gap = if total + keys.len() + 1 <= area.width as usize { 1 } else { 0 };
    let mut x = area.x + gap;
    for (key, name, action, active) in keys {
        let width = (key.len() + name.len() + 3) as u16;
        if x + width > area.x + area.width {
            break;
        }
        let bg = if active { ON_BG } else { DIV };
        let buf = f.buffer_mut();
        buf.set_string(x, area.y, " ", Style::new().bg(bg));
        buf.set_string(x + 1, area.y, key, Style::new().fg(HI).bg(bg).add_modifier(Modifier::BOLD));
        buf.set_string(x + 1 + key.len() as u16, area.y, format!(" {name} "), Style::new().fg(MAIN_FG).bg(bg));
        hits.areas.push((Rect { x, y: area.y, width, height: 1 }, action));
        x += width + gap;
    }
}
