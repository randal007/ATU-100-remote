# Project notes (VE7NHW)

Status and decisions for this branch. The tuner lives inside a homemade
amplifier run by an Arduino Mega; the goal is one firmware with the display,
all three buttons and a serial link to the Mega and/or an ESP32.

## Where things stand (2026-10-22)

- Firmware on branch `display-buttons-uart`: display + buttons + serial,
  bug fixes, new tuning search, Power/Antenna/Efficiency, `{"Raw":true}`,
  live settings, `HighSWR` event. 14835 of 16384 words.
- Tested only in the PC simulator (`test/run.sh`): 9 runs x 24 checks pass.
  **Never flashed or tried on air yet.**
- Programmer: PICkit 3, which needs MPLAB X / IPE v6.20 or older.
- Next: back up the tuner's eeprom, flash, check display/buttons/serial,
  then compare the two tuning searches on the real antennas (cell 0x36).

## Hardware

- RA6/RA7 (PIC pins 9 and 10) are not routed on N7DDC's EXT PCB; solder fine
  wire to the pins. 10k pull-up on RA6 (to the partner's logic voltage).
  Picture: `docs/atu100_ra6_ra7.png`. README.md is the manual. A photo of the actual board would
  confirm it is not a clone with a different layout.
- PROG header (MCLR, +5V, GND, DAT, CLK) doubles as the display connector.
- The tuner's 5 V is a 78L05 (100 mA): power nothing extra from it.

## ESP32 plan (not started)

- Board: Seeed XIAO ESP32-S3 (U.FL external antenna, native USB, 3 UARTs).
  Full size alternative: ESP32-S3-DevKitC-1U.
- Power: **7805 linear regulator from the tuner's 12 V input** (decided
  2026-10-22: no switching noise next to the SWR detector). 0.1 uF + 10 uF
  on each side, ferrite + 10 uF on the 12 V tap, output to the XIAO's 5 V
  pin. It dissipates about 7 V x current: ~0.6-1 W average (80-150 mA),
  ~2.5 W during Wi-Fi bursts, so bolt the tab (ground) to the chassis or
  a small heatsink; bare it would run ~90-100 C in a warm case. Rejected:
  Pololu D24V5F5 buck (efficient but switching), 7-36 V industrial
  ESP32-S3 boards (credit-card sized), the tuner's own 78L05 (100 mA).
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
