# Project notes (VE7NHW)

Status and decisions for both branches. The tuner lives inside a homemade
amplifier run by an Arduino Mega; the goal is one firmware with the display,
all three buttons and a serial link to the Mega and/or an ESP32.
README.md is the full manual; this file is the working notes.

## Where things stand (2026-10-24) — resume here

Two branches of randal007/ATU-100-remote, both pushed, both with every
fix from both code reviews. **Everything is tested in the simulators
only: nothing has been flashed or used on the air.**

| | `display-buttons-uart` | `tune-research` |
|---|---|---|
| Pull request | #1, into master (ukoda's, untouched) | #2, into display-buttons-uart |
| Tuning search | N7DDC's and the log grid (cell 36 chooses) | **the model search** only (cell 36 unused) |
| Firmware | 1.0.1: 15616 of 16384 words, 656 bytes RAM, stack 14 of 16 | 1.1.1: 16364 of 16384 words (**20 free**), 915 bytes RAM, stack 15 of 16 |
| Sweep, tuner as built, 10 W | log grid 91%, N7DDC 52% below SWR 1.5 | 96% |
| atutop, Python tools | have the search switch (`g`, `--search`) | no search switch |

This file is on both branches; this copy is on `display-buttons-uart`; the work goes on in `tune-research` (`git checkout tune-research`).

His decisions (2026-10-24): **`tune-research` goes forward**; **keep
both branches** (he flashes each to compare the searches on the real
tuner; never delete `display-buttons-uart`); what ends up in master is
open. Shared fixes go on `display-buttons-uart` first, then
`git merge display-buttons-uart` into `tune-research` (conflicts: main.h's
search code, cell 36, the README's tuning text and figures, the hex).

**The model search** (`tune-research`, main.h `tune_model`): measure 7
settings (no relays, L alone, C alone), fit a weighted parabola along the
L-only and C-only lines (s = (1+|g|²)/(1−|g|²) is exactly a parabola),
measure at their bottoms, calculate the antenna (R, X, frequency) and the
exact L-network match on both capacitor sides, refine with a 3x3 bowl fit,
and a 7x7 grid + bowl when that ends above SWR 1.3. 96% below SWR 1.5 in
1.5 s (log grid 91% / 3.7 s, N7DDC 52% / 1.4 s); times include the PIC's
calculating (0.43 s), measured in gpsim. Floats run on a byte-code
calculator (`m_calc`, programs `p_*` in main.h): a float op costs a PIC16
20-40 words as plain C; get_pwr/show_pwr maths moved onto it too. **Code
space is the constraint**: new firmware code on this branch must make room
(see the trims in research/tuning/README.md). The study, every method
tried and the tables: research/tuning/README.md. `research/tuning/firmware.py`
is the search in Python (reads the programs from main.h; keep it in step
with any change to the search), `research/tuning/cycles/run.sh` times the
programs in gpsim (16F1788, same core; Arch's gpsim package unpacked
anywhere works, `GPSIM=` points at it).

Both branches:

- **Tuner firmware** (PIC16F1938): display + 3 buttons + serial on RA6/RA7
  (pins 9/10), JSON protocol (one message per line; numbers 0-255, else
  Error), HighSWR (cell 37) and HighPower (from cell 31) alarms with the
  reading in the event, Raw detector readings, live settings, Reset/Tune
  stop a tune at any reading (Status/Raw answered, the rest Busy), serial
  robustness (128 byte buffer, damaged or overflowed messages dropped),
  ignores `{"ESP32":...}`, RelayI/RelayC in the status, letters only
  alone on a line. Revision `FW_REVISION` in main.c, in the Startup
  `Build` field. Hex: `ATU-100_display_uart_PIC16F1938_<date>.hex` in the
  repo root (rebuild + copy with every firmware change; build.sh checks
  the call stack).
- **Simulator**: `test/run.sh`: 43 checks per display type (40 LED) x 3
  baud offsets, ±2.5% for one display, then the search sweep (REAL=0 ideal
  relays, REAL=1 a tuner as built; FWD_MV=636 for 5 W; sweep CSVs with a
  `readings` column); `test/python_tools.sh` for the Python tools.
- **ESP32 bridge** (`esp32/atu100_bridge`, Seeed XIAO ESP32S3 **Plus**):
  USB + TCP 7000 bridge, UDP 7001 find, mDNS atu100.local, Mega port
  (JSON lines only), Wi-Fi settings over USB or from its hotspot,
  hotspot ATU100-setup / hamradio / 192.168.73.1 (always-on option),
  WebSocket 81 + atutop's browser files (web_files.h, gitignored), /lite,
  /setup. Compiles (58% of 2 MB with the browser files). **Untested on
  hardware.** Simulator: `test/esp32_sim.py` (reference for the messages;
  web 8080 / WS 8081; ATU100_SIM_LAN=1 for phones).
- **Mega sketch** (`arduino/atu100_bridge`): compiles for the Mega
  (`arduino-cli compile -b arduino:avr:mega`, core installed 2026-10-24).
- **atutop** (`tui/`, Rust): terminal (`native.rs`) and browser
  (`web.rs`, Ratzilla) share `app.rs` + `ui.rs` + `cells.rs`. Connect
  screen, network settings (o), ATU-100 settings (e: every EEPROM cell,
  measured L/C, defaults), winding dialog, HighPower scale, web indicator,
  phone layout and keyboards, scroll bar. Commands wait for their own
  reply. Installed at `~/.local/bin/atutop` (build of `tune-research`).
  `cargo test`: the cell encodings and the command pacing.
- **Python tools** updated; ignore ESP32 lines.

### Next

1. **First flash** (he is getting ready to test): back up the tuner's
   EEPROM and flash with the PICkit 3 (MPLAB IPE 6.20 or older, can be
   installed for him), check display/buttons/serial with atutop, then tune
   on the real antennas, each branch's hex in turn. Advise tuning with
   5-10 W (below ~2 W every search stops short: the reflected diode's knee).
2. **ESP32 bridge on the board** when the parts arrive: `esp32/build.sh`,
   upload; check opening the native USB port does not reset it (DTR/RTS),
   Wi-Fi join, hotspot + setup page, atutop over USB, TCP and the browser,
   the Mega port; and from the second review: a shared Wi-Fi scan, the
   hotspot lingering 30 s after always-on is turned off, no background
   retries after a failed join.
3. **Code reviews** (ultra, cloud), all findings answered on the PRs:
   - first, PR #1, 2026-10-23: 15 inline + 5 small, all fixed except
     item 11 (serial sampling point: kept, margin +3.5/−2.5% covers the
     PIC's 2%);
   - second, 2026-10-24: 32 inline + ~20 small on PR #1, 7 on PR #2, all
     fixed except: the display type's two copies (cannot disagree); the
     web-origin and hotspot-subnet checks (design questions for a LAN
     box); the simulator's timing caveats (notes).
4. Ideas: a raw "any cell" hex editor in ATU-100 settings (offered, not
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
- N7DDC's EXT manual (docs/): cell 31 tandem match ratio (1:10 to ~150 W,
  1:5 to ~40 W and better at 1-5 W, 1:32 + cell 30 to 1500 W; keep the
  PIC's inputs under 4.096 V); board rated 100 W; cell 05 minimum tune
  power, default 5 W.
