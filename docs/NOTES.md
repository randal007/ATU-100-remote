# Project notes (VE7NHW)

Status and decisions for this branch. The tuner lives inside a homemade
amplifier run by an Arduino Mega; the goal is one firmware with the display,
all three buttons and a serial link to the Mega and/or an ESP32.
README.md is the full manual; this file is the working notes.

## Where things stand (2026-10-24) — resume here

Two branches of randal007/ATU-100-remote, both tested in the simulators
only (**nothing flashed or used on the air**):

- `display-buttons-uart` (the fork's default, PR #1 into master): N7DDC's
  search and the log grid search (cell 36 chooses), firmware 1.0.
- `tune-research` (from it, his idea 2026-10-23: "one branch with the log
  and N7DDC, one with N7DDC and the better one"; then "get rid of N7DDC
  for this build too"): **the model search only**, firmware 1.1. Measure
  7 settings, fit a parabola along the L-only and C-only lines, measure at
  their bottoms, calculate the antenna and the exact match on both sides,
  refine with a 3x3 bowl fit, 7x7 grid + bowl above SWR 1.3. As built,
  10 W: 96% below SWR 1.5 in 1.5 s, against log grid 91% / 3.7 s and
  N7DDC 52% / 1.4 s. Floats run on a byte-code calculator (`m_calc`) to
  fit: 16325 of 16384 words, 919 bytes RAM, stack 15 of 16. Cell 36
  unused; atutop and the Python tools lost the search switch. The whole
  study, every method and the tables: research/tuning/README.md.
  `research/tuning/firmware.py` is the firmware's search in Python (reads
  the programs from main.h); `research/tuning/cycles/run.sh` times them
  in gpsim (16F1788, same core).

He chooses which branch goes forward after comparing; then the final
ultra review.

The rest of this section describes `display-buttons-uart`; on
`tune-research` read "model search" for "log grid search".

- **Tuner firmware** (PIC16F1938): display + 3 buttons + serial on RA6/RA7
  (pins 9/10), JSON protocol, log grid tuning search (cell 36), HighSWR
  (cell 37) and HighPower (from cell 31) alarms with the reading in the
  event, Raw detector readings, live settings, Busy while a tune waits,
  serial robustness (128 byte buffer, damaged messages dropped), ignores
  `{"ESP32":...}`, RelayI/RelayC in the status, letters only alone on a
  line. Revision `FW_REVISION` "1.0" in main.c, in the Startup `Build`
  field. 15535 of 16384 words (95%), 662 bytes RAM, call stack 14 of 16
  (build.sh checks it). Hex:
  `ATU-100_display_uart_PIC16F1938_20261024.hex` (rebuild + copy with
  every firmware change).
- **Simulator**: `test/run.sh`, 40 checks per display type (37 LED) x 3
  baud offsets, +-2.5% for one display, + the search sweep (log grid 91% vs
  N7DDC 45% below SWR 1.5); `test/python_tools.sh` for the Python tools.
- **ESP32 bridge** (`esp32/atu100_bridge`, Seeed XIAO ESP32S3 **Plus**):
  USB + TCP 7000 bridge, UDP 7001 find, mDNS atu100.local, Mega port
  (JSON lines only), Wi-Fi settings over USB or from its hotspot,
  hotspot ATU100-setup / hamradio / 192.168.73.1 (always-on option),
  WebSocket 81 + atutop's browser files (web_files.h, gitignored), /lite,
  /setup. Compiles (58% of 2 MB with the browser files). **Untested on
  hardware.** Simulator: `test/esp32_sim.py` (reference for the messages;
  web 8080 / WS 8081; ATU100_SIM_LAN=1 for phones).
- **atutop** (`tui/`, Rust): terminal (`native.rs`) and browser
  (`web.rs`, Ratzilla) share `app.rs` + `ui.rs` + `cells.rs`. Connect
  screen, network settings (o), ATU-100 settings (e: every EEPROM cell,
  measured L/C, defaults), winding dialog, HighPower scale, web indicator,
  phone layout and keyboards, scroll bar. Installed at
  `~/.local/bin/atutop`. `cargo test` checks the cell encodings.
- **Python tools** updated; ignore ESP32 lines.

### Next

1. **Which branch**: he compares `tune-research` (model search) with
   `display-buttons-uart`; then the final ultra review of the chosen one.
2. **First flash** (he is getting ready to test): back up the tuner's
   EEPROM and flash with the PICkit 3 (MPLAB IPE 6.20 or older, can be
   installed for him), check display/buttons/serial with atutop, then
   tune on the real antennas (with display-buttons-uart, cell 36 switches
   the search).
3. **ESP32 bridge on the board** when the parts arrive: `esp32/build.sh`,
   upload; check opening the native USB port does not reset it (DTR/RTS),
   Wi-Fi join, hotspot + setup page, atutop over USB, TCP and the browser,
   the Mega port.
4. **Code review done** (ultra, cloud, on PR #1 into master, 2026-10-23):
   15 inline findings + 5 small, all real, all fixed except item 11
   (serial sampling point: kept, margin +3.5/-2.5% covers the PIC's 2%;
   explained to him). After the fixes: merge PR #1, make master the
   default branch again, delete display-buttons-uart (he agreed in
   principle; confirm).
5. Ideas: a raw "any cell" hex editor in ATU-100 settings (offered, not
   asked for); band memories on the Mega/ESP32 (restore L, C and the LC/CL
   side); a hint in atutop that LC means a load above 50 ohm; report the
   Ratzilla resize bug upstream; move off the Ratzilla git commit once a
   release has the size fix.

### Trying atutop without the tuner

Run `atutop` and pick **simulator** (or **simulator + phone**) on the
connect screen: it starts `test/host_test` and `test/esp32_sim.py` and
stops them when atutop quits or its window closes. By hand:

    cd ATU_100_EXT_board/FirmWare_PIC16F1938/test
    ./run.sh                                  # builds host_test
    PTY=1 QUIET=1 RF_WAVE=1 ./host_test &     # prints the pty; also in ./pty_name
    ln -sfn "$(cat pty_name)" /tmp/atu100-sim
    atutop /tmp/atu100-sim

Stop it with `pkill -x host_test` (not `pkill -f`, which matches the shell
running it). The simulated Wi-Fi network VE7NHW-shack has the password
hamradio.

## Decisions and findings, by topic

### Tuner firmware

- Serial on RA6/RA7: not routed on N7DDC's EXT PCB (checked his gerbers),
  solder to the pins; RA6 needs a pull-up.
- Power: the board is rated 100 W; the tandem match winding (cell 31) sets
  what the detector can read (150 W at 1:10, ~37 W at 1:5; power goes
  with the ratio squared). HighPower fires above 100 W at 1:9 and up, else
  at 1.4 x K^2 W (35 W at 1:5, because a 40 W alarm could never fire before
  the detector's Overload); off in high power mode (cell 30). His
  decisions: 0-100 W scale and alarm for 1:10, 0-40 W for 1:5; "Tunes from"
  (cell 05) stays a separate setting (a 1 W / 5 W toggle in atutop), and
  the winding change goes through a warning box that offers the matching
  "Tunes from".
- Bugs he found in the simulator, all real and fixed: the HighSWR event
  carried no value (now `"SWR"`, HighPower `"Power"`); an auto tune started
  before the status with the high SWR went out (tune_btn_push sends it
  first); the status after RelayI/RelayC/bypass carried the old SWR
  (quick_swr takes a reading first). Below 1 W the SWR is 0.00 by design.
- LC/CL: one relay puts the capacitors at the radio end (CL, loads below
  50 ohm) or the antenna end (LC, above 50 ohm); every tune tries both;
  saved in cell FD.

### ESP32 bridge

- Board: Seeed XIAO ESP32S3 Plus (16 MB flash, 8 MB PSRAM). Power: a
  ready-made **LM7805 linear module** from the tuner's 12 V (no switching
  noise near the detector; not "7805 replacement" switchers like Recom
  R-78), ~1 W, heatsink or chassis, ferrite + 10 uF on the 12 V tap,
  output to the XIAO's 5 V pin. Not the tuner's 78L05 (100 mA).
- Wiring (README table): D5 (GPIO6) RX <- RA7 through 2.2k/3.3k; D4
  (GPIO5) TX -> RA6 (10k pull-up to 3.3 V); D8 RX <- Mega TX1 through a
  divider; D9 TX -> Mega RX1. Not D6/D7 (GPIO43/44): UART0, the S3's boot
  text goes out there (review item 5). Pull-ups with gpio_pullup_en, not
  pinMode (core 3.x detaches the UART). Tuner lines up to 3000 chars (the
  dump is ~2560), 4 KB TX buffers on USB and the Mega port.
- His decisions: Wi-Fi settings change over USB or from the hotspot only;
  hotspot fallback for off-grid use; password `hamradio`, address
  192.168.73.1; always-on option; no captive portal and no custom hotspot
  password for now. The hotspot and home network share one radio (the
  hotspot follows the home channel), so no network retries while someone
  is on the hotspot.
- Protocol: flat JSON, one line per message, only when asked: `Info`,
  `Scan` (`Net` lines, then `Scan`), `WiFi` (SSID, Pass omitted keeps the
  saved one, IP "" = DHCP, Gateway, Mask), `Forget`, `Hotspot` (Always);
  `OK`/`Error`; UDP 7001 `Find` -> `Here`. Clients on the hotspot subnet
  are `"Client":"Hotspot"`.

### atutop

- Commands go through an outbox: one at a time after an answer (or 1 s),
  repeats of the same setting merged, queued and just-sent commands
  re-applied after each incoming line (quick clicks stay right); while
  tuning only Reset, Status, Raw and the bridge's own go.
- Browser version: Ratzilla is pinned to git rev 2f6f7ef (0.3.1's
  DomBackend::size() guessed window/10 x /20). Ratzilla's own listeners sit
  on its grid, which it replaces on a resize, so web.rs listens on the
  document (pointer events) and finds the cell from the <span> under the
  pointer. Its resize handler empties its parent element, so it draws
  into <div id="screen"> (grid id `screen_ratzilla_grid`) and the phone
  text boxes (one real <input> per visible field) live outside it.
  Viewport `interactive-widget=resizes-visual`; 100dvh.
- Tests of the browser version: puppeteer from
  ~/Work/gods-eye-view/node_modules driving /usr/bin/chromium (desktop and
  a Pixel-like phone with touch); he has tested on his Android phone.
- The PC's firewall (ufw) lets 8080-8081 in from 192.168.88.0/24 for
  "simulator + phone" (rule added 2026-10-23 with his OK, comment
  "ATU-100 simulator web page (atutop)").

## Hardware

- RA6/RA7 (PIC pins 9 and 10) are not routed on N7DDC's EXT PCB; solder fine
  wire to the pins. 10k pull-up on RA6 (to the partner's logic voltage).
  Picture: `docs/atu100_ra6_ra7.png`. A photo of the actual board would
  confirm it is not a clone with a different layout.
- PROG header (MCLR, +5V, GND, DAT, CLK) doubles as the display connector.
- The tuner's 5 V is a 78L05 (100 mA): power nothing extra from it.
- PICkit 3: MPLAB X / IPE 6.20 or older (6.25 dropped it).

## Ideas parked

- Band memories kept on the Mega/ESP (the tuner cannot measure frequency):
  restore L/C and the side with RelayI/RelayC. Later.
- Exact millisecond delays from the Timer2 tick (delays now run ~20% long
  because of the serial interrupt). Undecided.
- MQTT from the ESP32 bridge. Not asked for yet.

## Research that led here

- Remote-control forks of the ATU-100: ukoda/ATU-100-remote (JSON, this
  fork's parent), edsonbrusque/ATU-100 (button letters only), dzl's blog zip
  (manual L/C and SWR, but no tuning). Add-ons that keep stock firmware:
  ptarsobastos/ATU_RemDisplay_V1, ftl/atu100wifi.
- pingpongshow/ATU-1000-ESP32-Controller replaces the PIC with an ESP32-S3
  hand-wired to the PIC footprint (3.3 V, needs ADC input clamps). Written
  for the ATU-1000, very new; only worth trying on a spare board.
- ukoda dropped the display by deleting its code, not for lack of pins or
  memory; his UART took over the Auto/Bypass button pins.
- N7DDC's tuning search: in the simulator it reaches SWR < 1.5 on about 46%
  of matchable loads (worst on the upper bands) because its coarse L steps
  are ~600 nH and it adjusts L and C one at a time. The log grid search
  reaches about 91% (99% of loads starting below SWR 10), ~3 s vs ~1 s.
- Tuning research (2026-10-23/24, research/tuning): the model search, 96%
  in 1.5 s on a realistic tuner; an unconstrained model fit reaches 98%
  but needs a minute of float maths on the PIC. Below ~2 W every search
  suffers (the reflected diode's knee): advise tuning with 5-10 W.
- The simulator's realistic tuner: `REAL=1` (host.c: parts off, strays,
  coil loss, diodes 15% off N7DDC's table); `FWD_MV=636` for 5 W; the
  sweep CSV has a `readings` column (get_pwr calls, each ~2 ms of float
  maths on the PIC that the simulator does not time).
- N7DDC's EXT manual (docs/): cell 31 tandem match ratio (1:10 to ~150 W,
  1:5 to ~40 W and better at 1-5 W, 1:32 + cell 30 to 1500 W; keep the
  PIC's inputs under 4.096 V); board rated 100 W; cell 05 minimum tune
  power, default 5 W.
