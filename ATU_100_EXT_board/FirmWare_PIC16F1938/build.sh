#!/bin/sh
# Command-line build without MPLAB X: ./build.sh [extra xc8 flags]
# Uses the same options as nbproject/Makefile-default.mk.
# XC8 and DFP locations can be overridden with XC8 / DFP environment variables.
set -e
cd "$(dirname "$0")"

XC8=${XC8:-$HOME/opt/microchip/xc8/v3.10/bin/xc8-cc}
DFP=${DFP:-$HOME/opt/microchip/packs/PIC12-16F1xxx_DFP/1.10.270}
OUT=build/cli
mkdir -p "$OUT"

"$XC8" -mcpu=16F1938 -mdfp="$DFP/xc8" \
    -fno-short-double -fno-short-float -O2 -maddrqual=ignore -mwarn=-9 \
    -msummary=-psect,-class,+mem,-hex,-file -ginhx32 -Wl,--data-init \
    -mno-keep-startup -mno-osccal -mno-resetbits -mno-save-resetbits \
    -mno-download -mno-stackcall -mdefault-config-bits -std=c99 \
    -mstack=compiled:auto:auto -Wa,-a "$@" \
    -o "$OUT/atu100.hex" \
    main.c pic_init.c cross_compiler.c oled_control.c uart.c json.c

# The PIC16F1938 has a 16-level hardware stack and resets when it overflows
# (STVREN on). XC8 estimates the deepest call chain in the listing; the
# interrupt's figure includes the main program's. A PC simulator cannot see
# this, so the build stops here instead.
DEPTH=$(grep "Estimated maximum stack depth" "$OUT/atu100.lst" | awk '{print $5}' | sort -n | tail -1)
echo "    Call stack depth     $DEPTH of 16 (with the interrupt)"
if [ -z "$DEPTH" ] || [ "$DEPTH" -gt 16 ]; then
    echo "build.sh: call stack too deep for the PIC ($DEPTH > 16): it would reset" >&2
    exit 1
fi
