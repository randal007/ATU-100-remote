#!/bin/sh
# Run the python tools against the simulated tuner on a pseudo terminal:
# status, bypass, tune, detector voltages, settings, relays, EEPROM dump,
# hex to JSON and EEPROM restore. Needs pyserial. Run ./run.sh first.
cd "$(dirname "$0")"
PY=../../../python
TMP=$(mktemp -d)
fails=0

rm -f pty_name
PTY=1 QUIET=1 ./host_test > pty_log.txt 2>&1 &
SIM=$!
trap 'kill $SIM 2>/dev/null; rm -rf "$TMP"' EXIT
for i in $(seq 1 50); do [ -s pty_name ] && break; sleep 0.1; done
PORT=$(cat pty_name)
sleep 5 # boot and a first measurement

atu() { timeout 60 python3 $PY/atu100.py -p "$PORT" --logfile /dev/null "$@" 2>&1; }
expect() { # what, output, pattern
    if echo "$2" | grep -qE "$3"; then echo "  ok   $1"; else echo "  FAIL $1"; echo "$2" | sed 's/^/       /'; fails=$((fails + 1)); fi
}

expect "status" "$(atu)" "SWR: +2\.18"
expect "bypass on" "$(atu -c B)" "Bypass: +Enabled"
expect "bypass off" "$(atu -c b)" "Bypass: +Disabled"
out=$(atu -c t)
expect "tune events" "$out" "Event: +Tuned"
expect "tune result" "$out" "SWR: +1\.[0-4]"
expect "detector voltages" "$(atu -c v)" "Forward: +[0-9]+ mV"
atu --swr-alarm 2.5 > /dev/null
expect "HighSWR limit" "$(atu -c g -a 37)" "37 = 25"
atu --search original > /dev/null
expect "tuning search" "$(atu -c g -a 36)" "36 = 00"
atu -c C -r 0 > /dev/null
expect "capacitor relay on" "$(atu)" "Capacitance: +330 pF"
atu -c d -e "$TMP/dump.json" > /dev/null
expect "EEPROM dump" "$(python3 -c "import json; print(len(json.load(open('$TMP/dump.json'))))")" "^256$"
python3 $PY/atu100hextojson.py ../../../ATU-100_display_uart_PIC16F1938_*.hex "$TMP/hex.json" > /dev/null
python3 - "$TMP/hex.json" <<'EOF'
import json, sys
d = json.load(open(sys.argv[1]))
d['00'], d['01'] = '4e', '01'          # as a 1602 LCD tuner would have them
json.dump(d, open(sys.argv[1], 'w'))
EOF
expect "restore uploads" "$(atu -c u -e "$TMP/hex.json")" "Upload complete"
atu -c d -e "$TMP/after.json" > /dev/null
expect "restore matches" "$(python3 -c "
import json
a = json.load(open('$TMP/hex.json')); b = json.load(open('$TMP/after.json'))
print([k for k in a if a[k] != b[k]])")" "^\[\]$"

grep -q '"Event":"Error"' pty_log.txt && { echo "  FAIL the tuner answered Error"; fails=$((fails + 1)); }
echo "python tools: $([ $fails = 0 ] && echo PASSED || echo FAILED): $fails failed"
exit $fails
