//! atutop: a btop-style interface for the ATU-100 tuner with the
//! display-buttons-uart firmware, in a terminal or, served by the ESP32
//! bridge, in a web browser. The same code draws and runs both: app.rs and
//! ui.rs; native.rs and web.rs are the two front ends.
//!
//!     atutop [PORT]     PORT: /dev/ttyACM0, COM3, socket://host:port
//!                       without one, the connect screen

mod app;
mod connect;
mod esp;
mod tuner;
mod ui;

#[cfg(not(target_arch = "wasm32"))]
mod link;
#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    native::main();
    #[cfg(target_arch = "wasm32")]
    web::main();
}
