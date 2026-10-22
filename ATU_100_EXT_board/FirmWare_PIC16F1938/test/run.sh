#!/bin/sh
# Build the firmware for the PC and run the scenario against it:
# OLED, 1602 LCD and two-colour LED, with the serial partner's clock
# exact and 2% off in each direction.
set -e
cd "$(dirname "$0")"
CC="gcc -std=gnu11 -O1 -g -funsigned-char -I. -I.."
$CC -w -Dmain=fw_main -c ../main.c -o fw_main.o
$CC -w -c ../pic_init.c ../cross_compiler.c ../uart.c ../json.c
$CC -Wall -c host.c host_display.c
$CC -o host_test *.o -lm
rm -f *.o
status=0
for display in 5 1 0; do
    for err in 0 2 -2; do
        DISPLAY_TYPE=$display BAUD_ERR=$err ./host_test > "log_d${display}_e${err}.txt" || status=1
        grep -E "PASSED|FAILED" "log_d${display}_e${err}.txt" | sed "s/^ */display $display, baud $err%: /"
    done
done
exit $status
