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
    -mstack=compiled:auto:auto "$@" \
    -o "$OUT/atu100.hex" \
    main.c pic_init.c cross_compiler.c oled_control.c uart.c json.c
