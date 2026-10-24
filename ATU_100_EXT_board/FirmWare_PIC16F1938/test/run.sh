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
# The serial margin: the PIC's own clock is within 2%; 2.5% either way still works
for err in 2.5 -2.5; do
    DISPLAY_TYPE=5 BAUD_ERR=$err ./host_test > "log_d5_e${err}.txt" || status=1
    grep -E "PASSED|FAILED" "log_d5_e${err}.txt" | sed "s/^ */display 5, baud $err%: /"
done
# Tuning searches over ~1000 loads on 160-10 m with 1% detector noise
for mode in 0 1; do
    [ $mode = 0 ] && name="N7DDC original" || name="log grid search"
    echo "tuning sweep, $name:"
    TUNE_MODE=$mode NOISE=1 SWEEP=1 DISPLAY_TYPE=0 ./host_test | grep -E "tuned below|left at"
    cp sweep.csv "sweep_mode$mode.csv"
done
exit $status
