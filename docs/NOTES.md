# Project notes (VE7NHW)

Status and decisions for this branch. The tuner lives inside a homemade
amplifier run by an Arduino Mega; the goal is one firmware with the display,
all three buttons and a serial link to the Mega and/or an ESP32.

## Where things stand (2026-10-23) — resume here

- Firmware on branch `display-buttons-uart` (the fork's default branch):
  display + buttons + serial, bug fixes, log grid tuning search,
  Power/Antenna/Efficiency, `{"Raw":true}`, live settings, `HighSWR` event.
  15082 of 16384 words, 662 bytes RAM.
- Revision: `FW_REVISION` in main.c (now "1.0"), sent in the Startup
  board message as `"Build":"VE7NHW display+serial 1.0, <build date>"`;
  atutop shows it at the top. Raise it with each release.
- 2026-10-23 serial robustness: 128 byte receive buffer; the measuring
  loop breaks off when a message starts (counting `{`, not `}`: a command
  is acted on before its `}` is read); damaged messages are dropped (a `{`
  or line end inside a message resets the parser). Found because spinning
  the mouse wheel in atutop overflowed the old 64 byte buffer and a
  garbled message left the parser out of step (endless `Error` events,
  Auto stuck). The 7-message burst test now lands all 7 with no errors.
- 2026-10-23 later: a tune waiting for carrier no longer gives up on any
  message. `serial_during_tune()` (main.c, called from get_swr's wait loop
  in main.h) answers Status/Raw/`s`, stops on Reset/Tune/`r`/`t`, and
  answers the rest with a new `Busy` event. Simulator step 10 checks it.
- atutop (tui/, Rust + ratatui): btop-style terminal program; installed to
  ~/.local/bin/atutop. Commands go through an outbox: one at a time, each
  after an answer (or 1 s), repeats of the same setting merged; while
  tuning only Reset, Status and Raw go. After each incoming line the
  queued commands and those sent in the last second are applied again on
  screen (`Outbox::reapply`), so quick clicks no longer toggle from a stale
  state (8 quick steps down from 3.0 now end at 2.2). Error/Busy drop the
  sent ones.
- Python tools updated and fixed; `test/python_tools.sh` drives them.
- README.md is the full manual.
- Tested only in the PC simulator (`test/run.sh`, `test/python_tools.sh`).
  **Never flashed or tried on air yet.**

- 2026-10-23 evening: ESP32 bridge groundwork, all simulated.
  Board now **Seeed XIAO ESP32S3 Plus**. Decisions: Wi-Fi settings change
  over USB only; hotspot `ATU100-setup` (setup page 192.168.4.1) when no
  network is saved or it cannot join one. Bridge protocol (flat JSON, one
  line per message, only sent when asked; the tuner ignores `ESP32`
  messages, so the probe is harmless without a bridge):
  `{"ESP32":"Info"}` -> Info with Version, Client (USB/WiFi), WiFi
  (Off/Connecting/Connected/Failed), SSID, IP, RSSI, Host, Port, Hotspot,
  HotspotName, StaticIP, Gateway, Mask; `Scan` -> one `Net` line per
  network then `Scan` Count; `WiFi` SSID [Pass: omitted keeps the saved
  one] IP ("" = DHCP) Gateway Mask -> OK; `Forget` -> OK; errors as
  `{"ESP32":"Error","Text":...}`; UDP 7001 `Find` -> `Here` IP Port Host
  SSID. TCP bridge on port 7000. Reference: test/esp32_sim.py.
- atutop: connect screen (USB by VID, network search, recent ports,
  address, simulator start), settings box (`o`) with Wi-Fi and fixed IP,
  use Wi-Fi, auto reconnect. Files tui/src/connect.rs, esp.rs.

### Open, next session

1. Back up the tuner's EEPROM, flash with the PICkit 3 (MPLAB IPE 6.20 or
   older), check display/buttons/serial, compare the two tuning searches
   on the real antennas (cell 0x36).
2. ESP32 bridge firmware: written, compiles; try it when the board
   arrives (README "ESP32 bridge"). Check: opening the native USB port
   does not reset it; Wi-Fi join/hotspot/setup page; atutop over USB and
   Wi-Fi; Mega pass-through.
3. Not done yet: a screenshot of the new screens for the README.
4. **Power range follows the detector winding** (asked 2026-10-23, research
   N7DDC's 3.2 manual in docs/ first). His understanding: minimum tune
   power ("Tunes from", cell 05) and the tandem match winding (cell 31,
   turns ratio) go together: wound for ~1 W tuning the tuner tops out at
   about 40 W; wound for 5 W it handles 0-100 W. Wanted in atutop: power
   scale 0-40 W or 0-100 W to match, an over-power alarm above the limit,
   and changing the setting made deliberate: a prompt asking how the
   detector is wound, warning that the 1 W winding limits power to 40 W
   (the detector overloads).
   **Research done (N7DDC EXT manual, docs/, "Change of default
   settings" + specs):**
   - The winding is the tandem match (SWR bridge) transformer's turns
     ratio, **cell 31** (`e_c_K_Mult`, BCD), default 10 = 1:10 = measures
     up to about **150 W**. 1:5 = up to about **40 W**, "works better with
     a minimum power of 1-5 W". 1:32 + cell 30 (high power) = 1500 W.
     Rule: the detector voltage at the PIC must stay under 4.096 V at full
     power; measurable power goes with the ratio squared (150 x (5/10)^2
     = 37.5 W).
   - Minimum tune power is a separate cell, **05** (default 05 = 5 W). The
     firmware does not link them: going to 1 W means rewinding to 1:5 AND
     setting cell 31 = 05 AND cell 05 = 01.
   - Board rating: "maximum working throughput 100 W" (relays and parts),
     "maximum measured 150 W" with the stock 1:10. So with 1:10 the
     over-power warning is at 100 W (board), not 150 (detector).
   - The firmware already sends an `Overload` event when the ADC reaches
     its top (raw > 1000): that is the detector limit, ~150 W at 1:10,
     ~37 W at 1:5. An over-power warning at the board/winding limit is new.
   **Plan:** atutop power scale fixed by cell 31 (1:5 -> 0-40 W, 1:10 ->
   0-100 W with the 100-150 W band red); over-power alarm in the firmware
   (like HighSWR, new event `HighPower`, limit derived from cell 31 or a
   new cell) so the Mega/ESP32 see it too; atutop "detector winding"
   setting behind a confirm dialog that sets cells 31 and 05 together
   (1:10 = 5 W min, 100 W max; 1:5 = 1 W min, 40 W max) and warns that
   the setting must match the actual winding or every power and SWR
   reading is wrong.
   **His decisions (2026-10-23):** stock 1:10 winding: scale 0-100 W and
   the over-power alarm above 100 W (the board's rating, even though the
   detector reads to 150). 1:5 winding: maximum 40 W, alarm above 40 W.
   Keep "Tunes from" (cell 05) a separate setting, but when the winding
   changes, offer the matching value (1:10 -> 5 W, 1:5 -> 1 W). Make the
   winding change obvious and deliberate: a prompt that says in plain
   words the detector overloads above 40 W with the 1:5 winding, and that
   the setting must match how the transformer is actually wound. Also warn
   if "Tunes from" is set below 5 W while the winding is 1:10.
   **Done 2026-10-23 late:** firmware `HighPower` event (main.c
   power_alarm, limit from cell 31: 100 W at 1:9 and up, else 1.4 x K^2 =
   35 W at 1:5, because the 1:5 detector tops out near 37 W, so a 40 W
   alarm would never fire before Overload; off in high power mode);
   simulator check at ~120 W. atutop: fixed scale (100 W / 40 W), limit
   mark, HIGH POWER flash, Winding row -> confirm dialog (sets cells 31
   and optionally 05), wheel on Tunes from with a warning below 5 W at
   1:10. Tell him the 35 W choice.
   **Web server indicator (asked same night), done:** header shows
   "● web server online IP:port" from the bridge's Info (`Web`,
   `WebPort`), click opens it with xdg-open. The simulated bridge serves a
   stand-in page; the real ESP32 page is still to write. The simulated
   bridge takes the next free port if 7000 is in use.
   **Bugs he found in the simulator (2026-10-23), fixed, all real:**
   (1) HighSWR in atutop's log showed the old SWR (1.21): the event had no
   value; it now carries `"SWR"` (HighPower carries `"Power"`) and atutop
   handles Event after the other fields. (2) "Tunes before it shows the
   high SWR": button_proc() ran the auto tune before send_state(), so the
   status with the high SWR never went out; tune_btn_push() now sends it
   first. Also the status straight after RelayI/RelayC/bypass carried the
   old relays' SWR (and antenna/efficiency): quick_swr() takes one reading
   first. Simulator steps 16-19 check an auto tune after a relay change
   (verified to fail on the old firmware). (3) "No power" next to 0.6 W:
   the firmware reports SWR 0.00 below 1 W on purpose (N7DDC); atutop now
   says "under 1 W: not measured".
   **ESP32 bridge firmware written** (esp32/atu100_bridge, compiles for
   esp32:esp32:XIAO_ESP32S3_Plus with arduino-cli 1.5.1 in
   ~/opt/arduino-cli, ArduinoJson 7.4.3; 45% flash). Untested on hardware.
   Web indicator: atutop also checks from the PC that the page is
   reachable (yellow "not reachable from here" otherwise).
   **Hotspot decisions (2026-10-23):** password `hamradio`, address
   192.168.73.1, an "always on" option (atutop settings, USB only,
   `{"ESP32":"Hotspot","Always":true}`); no captive portal, no custom
   hotspot password for now. Web page and port 7000 work on the hotspot
   (off-grid use); no network retries while a station is on the hotspot
   (the shared radio would move channel and drop it); mDNS from power-up;
   Info reports Web true on the hotspot and HotspotIP. Done in firmware,
   simulator and atutop.
   **Next: the web version of atutop** (he likes the plan): split tui/
   into a shared crate (ui.rs, tuner.rs, esp.rs, outbox, actions) and two
   shells, terminal and Ratzilla (ratatui/ratzilla 0.3.1, same ratatui
   0.30, mouse events with grid positions). ESP32: WebSocket that passes
   lines like port 7000, serve the wasm files from flash. For phones: a
   real HTML <input> placed over a focused field (phone keyboards send no
   keydown; Ratzilla listens to keydown on the document), and a narrow
   layout (~45 columns, boxes stacked) in the shared ui.
5. LC/CL explained to him (2026-10-23): one relay puts the capacitors at
   the radio end (CL) or the antenna end (LC); C goes on the high
   impedance side; every tune picks it; saved in cell FD. Band memories
   must restore it along with L and C. Maybe add a one-line hint in
   atutop ("C at antenna: load above 50 ohm").

### Trying atutop without the tuner

    cd ATU_100_EXT_board/FirmWare_PIC16F1938/test
    ./run.sh                                  # builds host_test
    PTY=1 QUIET=1 RF_WAVE=1 ./host_test &     # prints the pty; also in ./pty_name
    ln -sfn "$(cat pty_name)" /tmp/atu100-sim
    atutop /tmp/atu100-sim

Stop it with `pkill -x host_test` (not `pkill -f`, which matches the shell).

## Hardware

- RA6/RA7 (PIC pins 9 and 10) are not routed on N7DDC's EXT PCB; solder fine
  wire to the pins. 10k pull-up on RA6 (to the partner's logic voltage).
  Picture: `docs/atu100_ra6_ra7.png`. README.md is the manual. A photo of the actual board would
  confirm it is not a clone with a different layout.
- PROG header (MCLR, +5V, GND, DAT, CLK) doubles as the display connector.
- The tuner's 5 V is a 78L05 (100 mA): power nothing extra from it.

## ESP32 plan (not started; parts being ordered 2026-10-23)

- Board: **Seeed Studio XIAO ESP32S3, part number 113991114** (U.FL
  antenna included, native USB, 3 UARTs). Not the "Sense" version, which
  adds a camera. Full size alternative: ESP32-S3-DevKitC-1U.
- Power: **7805 linear regulator from the tuner's 12 V input** (decided
  2026-10-22: no switching noise next to the SWR detector). He is ordering
  a ready-made 7805 module with the capacitors on it. Check it is a real
  LM7805/L7805 (TO-220, linear): "7805 replacement" modules such as Recom
  R-78 are switching regulators. It drops about 1 W (0.6-1 W average,
  ~2.5 W in Wi-Fi bursts): heatsink or bolt to the chassis. Ferrite +
  10 uF on the 12 V tap, output to the XIAO's 5 V pin. Rejected: Pololu
  D24V5F5 buck (switching), 7-36 V industrial ESP32-S3 boards (big), the
  tuner's own 78L05 (100 mA).
- Wiring: tuner RA7 (5 V) -> 2.2k/3.3k divider -> ESP RX; ESP TX -> RA6
  direct, pull-up to 3.3 V; common ground.
- With the Mega too: ESP32 in the middle (tuner <-> ESP32 <-> Mega).
- Firmware wanted: USB serial bridge, Wi-Fi socket bridge
  (`socket://atu100.local:7000` for pyserial), web page with SWR/power/L/C
  and Tune/Bypass/Auto/Reset, Wi-Fi setup portal, optional MQTT.

## Ideas parked

- Band memories kept on the Mega/ESP (the tuner cannot measure frequency):
  restore L/C with RelayI/RelayC. Later.
- Exact millisecond delays from the Timer2 tick (delays now run ~20% long
  because of the serial interrupt). Undecided.

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
