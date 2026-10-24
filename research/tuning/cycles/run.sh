#!/bin/sh
# Time the calculator's programs on the PIC core, in instruction cycles.
# gpsim has no 16F1938; the 16F1788 has the same enhanced mid-range core,
# so the float routines take the same cycles. Needs gpsim (Arch: extra/gpsim;
# GPSIM=path/to/gpsim to use an unpacked copy) and XC8 as build.sh does.
set -e
cd "$(dirname "$0")"
FW=../../../ATU_100_EXT_board/FirmWare_PIC16F1938
XC8=${XC8:-$HOME/opt/microchip/xc8/v3.10/bin/xc8-cc}
DFP=${DFP:-$HOME/opt/microchip/packs/PIC12-16F1xxx_DFP/1.10.270}
GPSIM=${GPSIM:-gpsim}
# The calculator and its programs, straight from main.h
python3 - "$FW/main.h" <<'PY'
import sys
s = open(sys.argv[1]).read()
calc = s[s.index('/*  The calculator.'):s.index("// N7DDC's diode correction")]
progs = s[s.index('/*  Registers while measuring'):s.index('#define M_CACHE')]
near = s[s.index('static char m_nearest('):]
near = near[:near.index('\n}\n') + 3]
open('bench_calc.h', 'w').write(calc + '\n' + progs + '\n' + near)
PY
mkdir -p build
"$XC8" -mcpu=16F1788 -mdfp="$DFP/xc8" -fno-short-double -fno-short-float -O2 -std=c99 \
    -mstack=compiled:auto:auto -o build/bench.hex bench.c > build/xc8.log 2>&1 || { cat build/xc8.log; exit 1; }
printf 'break c 3000000\nrun\ndump\nquit\n' | $GPSIM -i -S disable -p p16f1788 build/bench.hex 2>&1 |
    grep -E "^00[234]0:" | python3 -c '
import sys
b = []
for line in sys.stdin:
    b += [int(x, 16) if x != "--" else 0 for x in line.split()[1:17]]
names = ("p_rho p_sig_read p_add p_fit p_vertex p_roots p_plus p_ant p_admit p_check p_lc_s p_lc "
         "p_cl_s p_cl p_target p_bowl p_power p_loss timer old_power old_loss m_nearest").split()
for i, n in enumerate(names):
    c = (b[2 * i] | b[2 * i + 1] << 8) * 8  # Timer1 at 1:8
    print(f"{n:12s} {c:7d} cycles {c / 4000:6.2f} ms at 16 MHz")
'
