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
2. ESP32 bridge firmware (Arduino core: USB CDC, UART to the tuner at
   4800, TCP 7000, UDP 7001, Preferences, mDNS, hotspot + setup page),
   following test/esp32_sim.py; mind the native USB reset on port open.
3. Not done yet: a screenshot of the new screens for the README.
4. **Power range follows the detector winding** (asked 2026-10-23, research
   N7DDC's 3.2 manual in docs/ first). His understanding: minimum tune
   power ("Tunes from", cell 05) and the tandem match winding (cell 31,
   turns ratio) go together: wound for ~1 W tuning the tuner tops out at
   about 40 W; wound for 5 W it handles 0-100 W. Wanted in atutop: power
   scale 0-40 W or 0-100 W to match, an over-power alarm above the limit,
   and changing the setting made deliberate: a prompt asking how the
   detector is wound, warning that the 1 W winding limits power to 40 W
   (the detector overloads). Check the facts against the manual and
   correct him where they differ.
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
