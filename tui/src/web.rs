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

/// Real text boxes over the settings fields, one per field, while the
/// settings box is open: a phone opens its keyboard only for a tap on a real
/// text box, so the tap must land on one. What is typed goes to the field.
struct Overlay {
    boxes: Vec<(Field, HtmlInputElement)>,
    focused: Option<Field>, // the field atutop had selected last frame
}

/// Ratzilla's grid, inside <div id="screen">: one <pre> per row, one <span> per cell
const GRID: &str = "screen_ratzilla_grid";

const FIELDS: [Field; 6] = [Field::Network, Field::Password, Field::Ip, Field::Gateway, Field::Mask, Field::AtuValue];

impl Overlay {
    fn new(core: Rc<RefCell<Core>>) -> Rc<RefCell<Overlay>> {
        let document = web_sys::window().unwrap().document().unwrap();
        let mut boxes = vec![];
        for field in FIELDS {
            let input: HtmlInputElement = document.create_element("input").unwrap().dyn_into().unwrap();
            input.set_class_name("atu_field");
            for (k, v) in [("autocomplete", "off"), ("autocapitalize", "off"), ("autocorrect", "off"), ("spellcheck", "false")] {
                input.set_attribute(k, v).unwrap();
            }
            let numeric = matches!(field, Field::Ip | Field::Gateway | Field::Mask | Field::AtuValue);
            input.set_attribute("inputmode", if numeric { "decimal" } else { "text" }).unwrap();
            document.body().unwrap().append_child(&input).unwrap();

            // A tap or click on the box: atutop's selected field follows.
            // (When atutop itself moved the focus, it is busy and knows.)
            let c = core.clone();
            let onfocus = Closure::<dyn FnMut()>::new(move || {
                if let Ok(mut c) = c.try_borrow_mut() {
                    c.act(Action::Focus(field));
                }
            });
            input.add_event_listener_with_callback("focus", onfocus.as_ref().unchecked_ref()).unwrap();
            onfocus.forget();

            // What is typed goes to the field
            let c = core.clone();
            let oninput = Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
                let Some(input) = e.target().and_then(|t| t.dyn_into::<HtmlInputElement>().ok()) else { return };
                if let Ok(mut c) = c.try_borrow_mut() {
                    c.act(Action::Focus(field));
                    c.set_field_text(&input.value());
                }
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
                    if let Ok(mut c) = c.try_borrow_mut() {
                        c.key(k);
                    }
                }
                e.stop_propagation();
            });
            input.add_event_listener_with_callback("keydown", onkey.as_ref().unchecked_ref()).unwrap();
            onkey.forget();
            boxes.push((field, input));
        }
        Rc::new(RefCell::new(Overlay { boxes, focused: None }))
    }

    /// Each frame: boxes over the fields on screen, the rest hidden; their
    /// text from atutop unless being typed in
    fn update(&mut self, core: &Core) {
        let document = web_sys::window().unwrap().document().unwrap();
        let active = document.active_element();
        let showing = core.text_boxes();
        let selected = core.selected_field();
        // Screen cells are spans in one <pre> per row
        let cell = |x: u16, y: u16| -> Option<web_sys::DomRect> {
            let grid = document.get_element_by_id(GRID)?;
            let row = grid.children().item(y as u32)?;
            Some(row.children().item(x as u32)?.get_bounding_client_rect())
        };
        let mut any_focused = false;
        for (field, input) in &self.boxes {
            let style = input.style();
            let is_active = active.as_ref().is_some_and(|e| e == input.unchecked_ref::<web_sys::Element>());
            let entry = showing.iter().find(|(f, _, _)| f == field);
            let place = entry.and_then(|(_, text, password)| {
                let rect = core.hits.areas.iter().find(|(_, a)| *a == Action::Focus(*field)).map(|(r, _)| *r)?;
                let (a, b) = (cell(rect.x, rect.y)?, cell(rect.x + rect.width - 1, rect.y)?);
                Some((text, *password, a, b))
            });
            let Some((text, password, a, b)) = place else {
                // The box with the keyboard stays while its field is still
                // showing, even if one frame did not draw it: hiding it
                // would put the phone's keyboard away
                if is_active && entry.is_some() {
                    continue;
                }
                if style.get_property_value("display").unwrap_or_default() != "none" {
                    let _ = style.set_property("display", "none");
                    let _ = input.blur();
                    input.set_value(""); // no password left behind in the page
                }
                continue;
            };
            let _ = style.set_property("display", "block");
            let _ = style.set_property("left", &format!("{}px", a.left()));
            let _ = style.set_property("top", &format!("{}px", a.top()));
            let _ = style.set_property("width", &format!("{}px", b.right() - a.left()));
            let _ = style.set_property("height", &format!("{}px", a.height()));
            let kind = if password { "password" } else { "text" };
            if input.type_() != kind {
                input.set_type(kind);
            }
            any_focused |= is_active;
            // atutop's text unless the box is being typed in; and the box
            // follows when atutop moves to another field (Tab, a network
            // picked from the list) while some box has the keyboard
            if !is_active && input.value() != *text {
                input.set_value(text);
            }
            if selected == Some(*field) && self.focused != Some(*field) && self.focused.is_some() {
                input.set_value(text);
                let _ = input.focus();
            }
        }
        self.focused = if any_focused || self.focused.is_some() { selected } else { None };
        if showing.is_empty() {
            self.focused = None;
        }
    }
}

fn map_key(e: &web_sys::KeyboardEvent) -> Option<Key> {
    let key = e.key();
    Some(match key.as_str() {
        "Enter" => Key::Enter,
        "Escape" => Key::Esc,
        "Tab" if e.shift_key() => Key::BackTab,
        "Tab" => Key::Tab,
        "Backspace" => Key::Backspace,
        "ArrowUp" => Key::Up,
        "ArrowDown" => Key::Down,
        k if k.len() > 1 && k.starts_with('F') => Key::F(k[1..].parse().ok()?),
        k if k.chars().count() == 1 => Key::Char(k.chars().next()?),
        _ => return None,
    })
}

/// The screen cell under the pointer: the character's own <span>, in its
/// row's <pre>, in Ratzilla's grid. Found from the page itself, so it holds
/// whatever the grid's size (Ratzilla builds a new grid on a resize).
fn cell_at(target: Option<web_sys::EventTarget>) -> Option<(u16, u16)> {
    let el: web_sys::Element = target?.dyn_into().ok()?;
    let span = el.closest(&format!("#{GRID} > pre > span")).ok()??;
    let index = |mut e: web_sys::Element| {
        let mut i = 0u16;
        while let Some(prev) = e.previous_element_sibling() {
            i += 1;
            e = prev;
        }
        i
    };
    let row = span.parent_element()?;
    Some((index(span), index(row)))
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

    let backend = DomBackend::new_by_id("screen").expect("the page");
    let terminal = ratatui::Terminal::new(backend).expect("the screen");
    let overlay = Overlay::new(core.clone());

    // Keys, clicks and the wheel from the whole page, not Ratzilla's own
    // listeners: those sit on its grid, which it replaces when the window
    // changes size, and then nothing answers
    let document = web_sys::window().unwrap().document().unwrap();
    {
        let core = core.clone();
        let onkey = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |e: web_sys::KeyboardEvent| {
            if e.ctrl_key() || e.alt_key() || e.meta_key() {
                return; // the browser's own shortcuts
            }
            let typing_in_box = e.target().and_then(|t| t.dyn_into::<HtmlInputElement>().ok()).is_some();
            if typing_in_box {
                return; // the text box over a field handles its own keys
            }
            if let Some(key) = map_key(&e) {
                e.prevent_default(); // no scrolling on Space, no reload on F5, Tab stays here
                let mut c = core.borrow_mut();
                let fx = c.key(key);
                effect(fx, &mut c);
            }
        });
        document.add_event_listener_with_callback("keydown", onkey.as_ref().unchecked_ref()).unwrap();
        onkey.forget();
    }
    {
        let (core, overlay) = (core.clone(), overlay.clone());
        let onmouse = Closure::<dyn FnMut(web_sys::MouseEvent)>::new(move |e: web_sys::MouseEvent| {
            if e.button() != 0 {
                return;
            }
            let Some((x, y)) = cell_at(e.target()) else { return };
            let mut c = core.borrow_mut();
            let fx = c.mouse(Mouse::Click(x, y));
            effect(fx, &mut c);
            overlay.borrow_mut().update(&c);
        });
        document.add_event_listener_with_callback("mousedown", onmouse.as_ref().unchecked_ref()).unwrap();
        onmouse.forget();
    }
    {
        let core = core.clone();
        let onwheel = Closure::<dyn FnMut(web_sys::WheelEvent)>::new(move |e: web_sys::WheelEvent| {
            let Some((x, y)) = cell_at(e.target()) else { return };
            let m = if e.delta_y() < 0.0 { Mouse::WheelUp(x, y) } else { Mouse::WheelDown(x, y) };
            let mut c = core.borrow_mut();
            let fx = c.mouse(m);
            effect(fx, &mut c);
        });
        document.add_event_listener_with_callback("wheel", onwheel.as_ref().unchecked_ref()).unwrap();
        onwheel.forget();
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
        overlay.borrow_mut().update(&c);
    });
}
