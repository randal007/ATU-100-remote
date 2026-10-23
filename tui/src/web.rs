//! The browser version: Ratzilla draws the same screen as the terminal
//! program into the page, and a WebSocket to the ESP32 bridge carries the
//! lines. Everything else is in app.rs and ui.rs, shared with the terminal.
//!
//! Phones: their keyboards appear only for a real text box, and they send
//! no key presses, so while a settings field is being typed in, a real
//! <input> sits exactly over it and passes what is typed to the field.

use crate::app::{Core, Effect, Field, Key, Mouse};
use crate::tuner::Kind;
use crate::ui::Action;
use ratzilla::event::{KeyCode, MouseButton, MouseEventKind};
use ratzilla::{DomBackend, WebRenderer};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlInputElement, MessageEvent, WebSocket};
use web_time::Instant;

enum Incoming {
    Open,
    Line(String),
    Closed(String),
}

/// The WebSocket to the bridge, opened again when it drops
struct WebLink {
    url: String,
    ws: Option<WebSocket>,
    inbox: Rc<RefCell<VecDeque<Incoming>>>,
    closed_at: Option<Instant>,
    ever_open: bool,
}

impl WebLink {
    fn connect(&mut self) {
        self.closed_at = None;
        let Ok(ws) = WebSocket::new(&self.url) else {
            self.closed_at = Some(Instant::now());
            return;
        };
        let inbox = self.inbox.clone();
        let onopen = Closure::<dyn FnMut()>::new(move || inbox.borrow_mut().push_back(Incoming::Open));
        ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
        onopen.forget();

        // Lines may arrive split over messages, or several in one
        let inbox = self.inbox.clone();
        let partial = Rc::new(RefCell::new(String::new()));
        let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            let Some(text) = e.data().as_string() else { return };
            let mut buf = partial.borrow_mut();
            buf.push_str(&text);
            while let Some(i) = buf.find('\n') {
                let line = buf[..i].trim().to_string();
                buf.replace_range(..=i, "");
                if !line.is_empty() {
                    inbox.borrow_mut().push_back(Incoming::Line(line));
                }
            }
        });
        ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        onmessage.forget();

        let inbox = self.inbox.clone();
        let onclose = Closure::<dyn FnMut()>::new(move || inbox.borrow_mut().push_back(Incoming::Closed("connection closed".into())));
        ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
        onclose.forget();
        self.ws = Some(ws);
    }

    fn send(&self, msg: &str) {
        if let Some(ws) = &self.ws {
            if ws.ready_state() == WebSocket::OPEN {
                let _ = ws.send_with_str(&format!("{msg}\n"));
            }
        }
    }
}

/// The bridge's WebSocket: the page's own host, on the port the page names
/// (the simulator puts its own in window.ATU_WS_PORT), else 81
fn ws_url() -> String {
    let window = web_sys::window().expect("window");
    let host = window.location().hostname().unwrap_or_default();
    let port = js_sys::Reflect::get(&window, &JsValue::from_str("ATU_WS_PORT"))
        .ok()
        .and_then(|v| v.as_f64())
        .map(|p| p as u16)
        .unwrap_or(81);
    format!("ws://{host}:{port}/")
}

/// A real text box over the settings field being typed in
struct Overlay {
    input: HtmlInputElement,
    field: Option<Field>,
}

impl Overlay {
    fn new(core: Rc<RefCell<Core>>) -> Rc<RefCell<Overlay>> {
        let document = web_sys::window().unwrap().document().unwrap();
        let input: HtmlInputElement = document.create_element("input").unwrap().dyn_into().unwrap();
        input.set_attribute("id", "atu_field").unwrap();
        input.set_attribute("autocomplete", "off").unwrap();
        input.set_attribute("autocapitalize", "off").unwrap();
        input.set_attribute("spellcheck", "false").unwrap();
        document.body().unwrap().append_child(&input).unwrap();
        let overlay = Rc::new(RefCell::new(Overlay { input: input.clone(), field: None }));

        // What is typed goes to the field
        let c = core.clone();
        let oninput = Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
            let Some(input) = e.target().and_then(|t| t.dyn_into::<HtmlInputElement>().ok()) else { return };
            c.borrow_mut().set_field_text(&input.value());
        });
        input.add_event_listener_with_callback("input", oninput.as_ref().unchecked_ref()).unwrap();
        oninput.forget();

        // Enter connects, Tab moves on, Escape closes, as in the terminal
        let c = core.clone();
        let onkey = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |e: web_sys::KeyboardEvent| {
            let key = match e.key().as_str() {
                "Enter" => Some(Key::Enter),
                "Escape" => Some(Key::Esc),
                "Tab" if e.shift_key() => Some(Key::BackTab),
                "Tab" => Some(Key::Tab),
                _ => None,
            };
            if let Some(k) = key {
                e.prevent_default();
                c.borrow_mut().key(k);
            }
            e.stop_propagation();
        });
        input.add_event_listener_with_callback("keydown", onkey.as_ref().unchecked_ref()).unwrap();
        onkey.forget();

        // A tap's mousedown puts the focus on the screen grid after our
        // handler has run; the click that follows puts it back on the box
        // (and, being the tap itself, brings up a phone's keyboard)
        let field = input.clone();
        let onclick = Closure::<dyn FnMut()>::new(move || {
            if field.style().get_property_value("display").unwrap_or_default() == "block" {
                let _ = field.focus();
            }
        });
        document.add_event_listener_with_callback("click", onclick.as_ref().unchecked_ref()).unwrap();
        onclick.forget();
        overlay
    }

    /// Put the box over the focused field, or hide it. `focus`: from a tap,
    /// where focusing brings up a phone's keyboard
    fn update(&mut self, core: &Core, focus: bool) {
        let style = self.input.style();
        let target = core.app.settings.as_ref().filter(|_| core.may_set_wifi()).and_then(|s| {
            let rect = core.hits.areas.iter().find(|(_, a)| *a == Action::Focus(s.field)).map(|(r, _)| *r)?;
            Some((s, rect))
        });
        let Some((s, rect)) = target else {
            if self.field.is_some() {
                let _ = style.set_property("display", "none");
                let _ = self.input.blur();
                self.field = None;
                // keys back to the screen
                if let Some(grid) = web_sys::window().unwrap().document().unwrap().get_element_by_id("grid") {
                    if let Ok(g) = grid.dyn_into::<web_sys::HtmlElement>() {
                        let _ = g.focus();
                    }
                }
            }
            return;
        };
        let document = web_sys::window().unwrap().document().unwrap();
        // Screen cells are spans in one <pre> per row: measure the field's
        // first cell and its last
        let cell = |x: u16, y: u16| -> Option<web_sys::DomRect> {
            let grid = document.get_element_by_id("grid")?;
            let row = grid.children().item(y as u32)?;
            Some(row.children().item(x as u32)?.get_bounding_client_rect())
        };
        let (Some(a), Some(b)) = (cell(rect.x, rect.y), cell(rect.x + rect.width - 1, rect.y)) else { return };
        let _ = style.set_property("display", "block");
        let _ = style.set_property("left", &format!("{}px", a.left()));
        let _ = style.set_property("top", &format!("{}px", a.top()));
        let _ = style.set_property("width", &format!("{}px", b.right() - a.left()));
        let _ = style.set_property("height", &format!("{}px", a.height()));
        let password = s.field == Field::Password && !s.show_pass;
        let _ = self.input.set_attribute("type", if password { "password" } else { "text" });
        let numeric = matches!(s.field, Field::Ip | Field::Gateway | Field::Mask);
        let _ = self.input.set_attribute("inputmode", if numeric { "decimal" } else { "text" });
        let text = match s.field {
            Field::Network => &s.ssid,
            Field::Password => &s.pass,
            Field::Ip => &s.ip,
            Field::Gateway => &s.gateway,
            Field::Mask => &s.mask,
        };
        // a new field, or a change from elsewhere (a network picked from the list)
        if self.field != Some(s.field) || self.input.value() != *text {
            self.input.set_value(text);
        }
        if focus || self.field != Some(s.field) {
            let _ = self.input.focus();
        }
        self.field = Some(s.field);
    }
}

fn map_key(k: &ratzilla::event::KeyEvent) -> Option<Key> {
    Some(match k.code {
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Tab if k.shift => Key::BackTab,
        KeyCode::Tab => Key::Tab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::F(n) => Key::F(n),
        _ => return None,
    })
}

/// The few things the terminal does that make no sense here
fn effect(e: Option<Effect>, core: &mut Core) {
    match e {
        Some(Effect::OpenWeb(url)) => {
            let _ = web_sys::window().unwrap().open_with_url(&url);
        }
        Some(Effect::SwitchTo(_)) => core.t.log("This page talks to the bridge it came from", Kind::Info),
        _ => {}
    }
}

pub fn main() {
    console_error_panic_hook::set_once();
    let core = Rc::new(RefCell::new(Core::new(true)));
    let link = Rc::new(RefCell::new(WebLink {
        url: ws_url(),
        ws: None,
        inbox: Rc::new(RefCell::new(VecDeque::new())),
        closed_at: None,
        ever_open: false,
    }));
    core.borrow_mut().t.log(format!("Connecting to {}", link.borrow().url), Kind::Info);
    link.borrow_mut().connect();

    let backend = DomBackend::new().expect("the page");
    let mut terminal = ratatui::Terminal::new(backend).expect("the screen");
    let overlay = Overlay::new(core.clone());

    {
        let core = core.clone();
        terminal
            .on_key_event(move |k| {
                if let Some(key) = map_key(&k) {
                    let mut c = core.borrow_mut();
                    let e = c.key(key);
                    effect(e, &mut c);
                }
            })
            .expect("keys");
    }
    {
        let (core, overlay) = (core.clone(), overlay.clone());
        terminal
            .on_mouse_event(move |m| {
                if let MouseEventKind::ButtonDown(MouseButton::Left) = m.kind {
                    let mut c = core.borrow_mut();
                    let e = c.mouse(Mouse::Click(m.col, m.row));
                    effect(e, &mut c);
                    // a tap on a field: the text box now, while the tap counts
                    overlay.borrow_mut().update(&c, true);
                }
            })
            .expect("mouse");
    }

    terminal.draw_web(move |f| {
        let mut c = core.borrow_mut();
        let mut l = link.borrow_mut();
        let url = l.url.clone();
        let incoming: Vec<Incoming> = l.inbox.borrow_mut().drain(..).collect();
        for msg in incoming {
            match msg {
                Incoming::Open => {
                    c.t.log(if l.ever_open { "Link back".to_string() } else { format!("Connected to {url}") }, Kind::Good);
                    c.connected(&url, !l.ever_open);
                    l.ever_open = true;
                }
                Incoming::Line(line) => c.line(&line),
                Incoming::Closed(why) => {
                    if l.closed_at.is_none() {
                        if c.app.link_error.is_none() && l.ever_open {
                            c.lost(why);
                        }
                        l.closed_at = Some(Instant::now());
                    }
                }
            }
        }
        if l.closed_at.is_some_and(|t| t.elapsed() >= Duration::from_secs(2)) {
            l.connect(); // try again
        }
        if c.app.port.is_empty() && !l.ever_open {
            c.app.link_error = Some("connecting…".into());
        }
        c.tick(&mut |m| l.send(m));
        c.draw(f);
        overlay.borrow_mut().update(&c, false);
    });
}
