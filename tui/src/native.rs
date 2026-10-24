//! The terminal program: crossterm for keys and the mouse, a serial port or
//! TCP for the link, the connect screen and the simulator. Everything else
//! is in app.rs, shared with the browser version.

use crate::app::{Core, Effect, Key, Mouse};
use crate::connect;
use crate::link::{Incoming, Link};
use crate::tuner::Kind;
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::crossterm::execute;
use std::io::stdout;
use std::time::{Duration, Instant};

const HELP: &str = "atutop: btop-style control of an ATU-100 tuner

usage: atutop [PORT]

PORT is a serial port (/dev/ttyACM0, COM3) or socket://host:port for a
network bridge; default $ATU100_PORT, else /dev/ttyACM0. 4800 baud.

Without a PORT it opens the connect screen: USB ports, ESP32 bridges found
on the network, and the ports used before (kept in ~/.config/atutop/recent).

keys: t tune, b bypass, a auto, r reset, s status, v detector voltages,
g tuning search, [ ] HighSWR limit, o network settings (ESP32 Wi-Fi),
e ATU-100 settings (the EEPROM cells), c connect
to another port, q or Esc quit. The mouse clicks the buttons, the relays
and the tuner settings; the wheel on HighSWR changes it.";

/// What the terminal program keeps besides the shared state
struct Shell {
    link: Option<Link>,
    switch_to: Option<String>,         // a port to move to
    sim: Option<connect::Simulator>,   // started from the connect screen
    web_check: Option<std::sync::mpsc::Receiver<Option<String>>>,
    last_retry: Instant,
    last_web: Option<Instant>, // None: check soon
    /// A port being opened in the background (a TCP connect can take
    /// seconds): the port, whether it is a new one, the result to come
    opening: Option<(String, bool, std::sync::mpsc::Receiver<Result<Link, String>>)>,
}

/// Try the web page's addresses from here, in the background; the first
/// that answers
fn check_web(urls: Vec<String>) -> std::sync::mpsc::Receiver<Option<String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::net::ToSocketAddrs;
        let reach = urls.into_iter().find(|url| {
            let addr = url.trim_start_matches("http://").trim_end_matches('/').to_string();
            let addr = if addr.contains(':') { addr } else { format!("{addr}:80") };
            addr.to_socket_addrs()
                .ok()
                .and_then(|mut a| a.next())
                .is_some_and(|a| std::net::TcpStream::connect_timeout(&a, Duration::from_secs(2)).is_ok())
        });
        let _ = tx.send(reach);
    });
    rx
}

pub fn main() {
    let arg = std::env::args().nth(1);
    if matches!(arg.as_deref(), Some("-h" | "--help")) {
        println!("{HELP}");
        return;
    }
    let port = arg.or_else(|| std::env::var("ATU100_PORT").ok());
    let mut core = Core::new(false);
    let mut shell = Shell {
        link: None,
        switch_to: None,
        sim: None,
        web_check: None,
        last_retry: Instant::now(),
        last_web: None,
        opening: None,
    };
    match port {
        Some(port) => match Link::open(&port) {
            Ok(l) => {
                shell.link = Some(l);
                connect::remember(&port);
                core.t.log("Connected, asking for the status and settings", Kind::Info);
                core.connected(&port, true);
            }
            Err(e) => {
                eprintln!("atutop: cannot open {e}");
                std::process::exit(1);
            }
        },
        None => core.app.connect = Some(connect::Connect::new("")),
    }

    let mut terminal = ratatui::init();
    let _ = execute!(stdout(), EnableMouseCapture);
    // ratatui's panic hook restores the terminal but not the mouse
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), DisableMouseCapture);
        hook(info);
    }));
    let result = run(&mut terminal, &mut core, &mut shell);
    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    if let Err(e) = result {
        eprintln!("atutop: {e}");
    }
}

fn run(terminal: &mut ratatui::DefaultTerminal, core: &mut Core, shell: &mut Shell) -> std::io::Result<()> {
    loop {
        while let Some(msg) = shell.link.as_ref().and_then(|l| l.rx.try_recv().ok()) {
            match msg {
                Incoming::Line(line) => core.line(&line),
                Incoming::Closed(why) => {
                    core.lost(why);
                    shell.last_retry = Instant::now();
                }
            }
        }
        reopen(core, shell);
        if let Some(c) = core.app.connect.as_mut() {
            c.poll();
        }
        // Can this computer reach the bridge's web page?
        if let Some(rx) = &shell.web_check {
            if let Ok(reach) = rx.try_recv() {
                core.app.web_reach = Some(reach);
                shell.web_check = None;
            }
        }
        let urls = core.app.esp.web_urls();
        if urls.is_empty() {
            core.app.web_reach = None;
        } else if shell.web_check.is_none() && shell.last_web.is_none_or(|t| t.elapsed() >= Duration::from_secs(10)) {
            shell.last_web = Some(Instant::now());
            shell.web_check = Some(check_web(urls));
        }
        let link = &mut shell.link;
        core.tick(&mut |msg| {
            if let Some(l) = link.as_mut() {
                l.send(msg)
            }
        });

        terminal.draw(|f| core.draw(f))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let effect = match event::read()? {
            Event::Key(k) if k.kind == KeyEventKind::Press => {
                let key = match k.code {
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => Some(Key::Esc),
                    KeyCode::Char(c) => Some(Key::Char(c)),
                    KeyCode::Enter => Some(Key::Enter),
                    KeyCode::Esc => Some(Key::Esc),
                    KeyCode::Tab => Some(Key::Tab),
                    KeyCode::BackTab => Some(Key::BackTab),
                    KeyCode::Backspace => Some(Key::Backspace),
                    KeyCode::Up => Some(Key::Up),
                    KeyCode::Down => Some(Key::Down),
                    KeyCode::F(n) => Some(Key::F(n)),
                    _ => None,
                };
                key.and_then(|k| core.key(k))
            }
            Event::Mouse(m) => {
                let mouse = match m.kind {
                    MouseEventKind::Down(MouseButton::Left) => Some(Mouse::Click(m.column, m.row)),
                    MouseEventKind::Drag(MouseButton::Left) => Some(Mouse::Drag(m.column, m.row)),
                    MouseEventKind::Up(MouseButton::Left) => Some(Mouse::Release),
                    MouseEventKind::ScrollUp => Some(Mouse::WheelUp(m.column, m.row)),
                    MouseEventKind::ScrollDown => Some(Mouse::WheelDown(m.column, m.row)),
                    _ => None,
                };
                mouse.and_then(|m| core.mouse(m))
            }
            _ => None,
        };
        match effect {
            Some(Effect::Quit) => return Ok(()),
            Some(e) => effect_do(e, core, shell),
            None => {}
        }
    }
}

/// The things only the terminal program does
fn effect_do(e: Effect, core: &mut Core, shell: &mut Shell) {
    match e {
        Effect::Quit => {}
        Effect::OpenWeb(url) => {
            let opener = if cfg!(target_os = "macos") { "open" } else if cfg!(windows) { "explorer" } else { "xdg-open" };
            let started = std::process::Command::new(opener)
                .arg(&url)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            match started {
                Ok(_) => core.t.log(format!("Opening {url}"), Kind::Info),
                Err(e) => core.t.log(format!("Cannot open a browser: {e}"), Kind::Bad),
            }
        }
        Effect::SwitchTo(port) => shell.switch_to = Some(port),
        Effect::OpenConnect => core.app.connect = Some(connect::Connect::new(&core.app.port)),
        Effect::ConnectRefresh => {
            if let Some(c) = core.app.connect.as_mut() {
                c.refresh();
            }
        }
        Effect::ConnectPick(i) => {
            if let Some(c) = core.app.connect.as_mut() {
                c.selected = i as usize;
                c.typing = false;
                shell.switch_to = c.chosen();
            }
        }
        Effect::ConnectGo => {
            if let Some(c) = core.app.connect.as_mut() {
                c.error = None;
                shell.switch_to = c.chosen();
            }
        }
    }
}

/// Moving to another port (USB to Wi-Fi, the connect screen, the
/// simulator), or trying again after the link was lost
fn reopen(core: &mut Core, shell: &mut Shell) {
    if let Some((port, moving, rx)) = shell.opening.take() {
        match rx.try_recv() {
            Ok(result) => opened(core, shell, port, moving, result),
            Err(std::sync::mpsc::TryRecvError::Empty) => shell.opening = Some((port, moving, rx)),
            Err(_) => opened(core, shell, port, moving, Err("could not open".into())),
        }
        return;
    }
    let retry = core.app.link_error.is_some() && !core.app.port.is_empty() && shell.last_retry.elapsed() >= Duration::from_secs(2);
    let Some(mut port) = shell.switch_to.take().or_else(|| retry.then(|| core.app.port.clone())) else { return };
    shell.last_retry = Instant::now();
    // The simulator: start it, then connect to its ESP32 bridge
    if port == connect::SIM || port == connect::SIM_LAN {
        let lan = port == connect::SIM_LAN;
        shell.sim = None; // a fresh one
        match connect::Simulator::start(lan) {
            Ok(sim) => {
                port = sim.port.clone();
                shell.sim = Some(sim);
                core.t.log("Simulator started: a pretend tuner and ESP32 bridge", Kind::Info);
                if lan {
                    core.t.log("Its web page opens to phones once the bridge is on Wi-Fi: see the top line", Kind::Info);
                }
            }
            Err(e) => {
                if let Some(c) = core.app.connect.as_mut() {
                    c.error = Some(e);
                }
                return;
            }
        }
    }
    let moving = shell.link.is_none() || port != core.app.port;
    // Let go of the old link first, also when trying the same port again:
    // its serial handle is exclusive, and a replugged board would otherwise
    // come back under another name
    shell.link = None;
    // Open in the background: the screen keeps answering while it connects
    let (tx, rx) = std::sync::mpsc::channel();
    let p = port.clone();
    std::thread::spawn(move || {
        let _ = tx.send(Link::open(&p));
    });
    shell.opening = Some((port, moving, rx));
}

fn opened(core: &mut Core, shell: &mut Shell, port: String, moving: bool, result: Result<Link, String>) {
    shell.last_retry = Instant::now();
    match result {
        Ok(new) => {
            shell.link = Some(new);
            core.t.log(if moving { format!("Connected to {port}") } else { "Link back".into() }, Kind::Good);
            if moving {
                connect::remember(&port);
                core.app.connect = None;
            }
            core.connected(&port, moving);
        }
        Err(e) if moving => {
            core.t.log(format!("Cannot connect to {e}"), Kind::Bad);
            match core.app.connect.as_mut() {
                Some(c) => c.error = Some(e),
                None => core.app.esp.notice = Some((format!("cannot connect: {e}"), true)),
            }
            if !core.app.port.is_empty() {
                // the old link was let go: try its port again, also while
                // the connect screen is still open
                core.app.link_error = Some("not connected".into());
            }
        }
        Err(e) => core.app.link_error = Some(e),
    }
}
