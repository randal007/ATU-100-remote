#!/bin/sh
# Build the ESP32 bridge with atutop's browser version inside:
#   1. atutop for the browser (tui/, trunk), 2. pack it into web_files.h,
#   3. compile the bridge. Upload with:
#   arduino-cli upload -b esp32:esp32:XIAO_ESP32S3_Plus -p /dev/ttyACM0 esp32/atu100_bridge
set -e
cd "$(dirname "$0")"
ARDUINO_CLI=${ARDUINO_CLI:-$HOME/opt/arduino-cli/arduino-cli}
PATH=$HOME/.cargo/bin:$PATH
(cd ../tui && trunk build --release)
./make_web_files.py
"$ARDUINO_CLI" compile -b esp32:esp32:XIAO_ESP32S3_Plus atu100_bridge
