# Tuning research: a faster, surer search for the ATU-100

How the firmware's tuning search (main.h, `tune_model`) was found, and
every method tried on the way. The short version is in the manual
(README.md, "Tuning search").

## The problem

The tuner only measures |Γ|: forward and reflected voltage from two diode
detectors. No phase, no frequency. Each setting tried costs a relay
settling wait (cell 03, 15 ms, about 14.5 ms with the serial interrupt's
slowdown) and a reading. N7DDC's search is quick but stops short on half
the loads; the log grid search of the `display-buttons-uart` build finds
91% but takes 3 to 4 s.

## The idea

With C = 0 the network adds only a series L to the antenna Z = R + jX, and
s = (1 + |Γ|²) / (1 − |Γ|²) is exactly a parabola in L:

    s = (R² + (X + ωL)² + 2500) / (100 R)

so c2 = ω² / (100 R), c1 = 2Xω / (100 R), and the parabola's lowest value
K satisfies R² − 100 K R + 2500 = 0 (two roots, R and 2500/R). With L = 0
the same holds for C in admittance. A few readings along each line give R,
X and ω, and from them the exact L-network match on either capacitor side.
The readings near a parabola's bottom fix it far better than the ones high
up its sides (s grows without bound as |Γ| → 1, and the diode's knee hides
small |Γ|), so the fit is weighted, and the search measures again where
the first fit puts the bottom.

## Files

| File | What |
|---|---|
| `lab.py` | the relay network, an antenna, the detector as `test/host.c` models it with `REAL=1` (parts a few % off, strays, coil loss, diodes with 15% more drop than N7DDC's table), the firmware's way of reading, and the time each step costs |
| `loads_real.csv` | the 1121 loads of the simulator's sweep that the relays can match below SWR 1.5 (from `host_test` with `REAL=1 SWEEP=1`) |
| `searches.py` | the log grid search (as in `display-buttons-uart`), DL8UG's ATU-10 NG search as his TUNING.md describes it, and the model searches tried (`solve` to `solve9`) |
| `model.py` | the antenna fit: full Nelder-Mead fits, the closed-form and weighted parabola estimates, the exact match |
| `pic.py` | the search shaped for the PIC (running sums, closed-form bowl, a small memory of readings, 32-bit float), and the trials that decided its final form |
| `firmware.py` | **the firmware's search, step for step**: it reads the calculator's programs out of main.h and runs them in 32-bit float, so it cannot drift from the firmware's formulas; with the PIC's calculating time from `cycles/` |
| `analyse.py`, `diag.py` | where a search fails (by band, worst loads); how far each estimate lands from the true best |
| `cycles/` | `run.sh`: the calculator's programs timed in instruction cycles in gpsim |

    python3 firmware.py                 # the firmware's search over all loads
    python3 firmware.py 14.2 10 -400    # one load, every reading
    ../../ATU_100_EXT_board/FirmWare_PIC16F1938/test/run.sh   # the real firmware, simulated

Python 3, no packages. The lab's time counts the relay waits (one per
group of relays changed) and 1 ms per reading. `firmware.py` adds the
calculating, from cycles measured in gpsim; the firmware simulator does not
count calculating at all.

## Every method compared

The lab, on the tuner as built, 10 W, 1% detector noise, the 1121 loads.
"Near best": within 0.1 of the best SWR the relays can reach, or below
1.25. Times are the mean (and 9 tunes in 10 within). Times marked * count
one relay wait for L and C changed together, as the new firmware does;
the others two.

| Method | Below 1.5 | Near best | SWR 2 or worse | Time | Fits the PIC? |
|---|---|---|---|---|---|
| N7DDC original (firmware simulator) | 52% | — | 38% | 1.3 s | yes (stock) |
| Log grid (earlier builds) | 91% | 84% | 4.6% | 3.3 s* (6.7) | yes |
| DL8UG ATU-10 NG style: 5×5 grid both sides, 3 best, pattern search | 88% | 85% | 7.9% | 1.6 s* (2.1) | yes |
| Full model fit (Nelder-Mead from a coarse grid), then refine | 94% | 92% | 4.3% | 0.4 s, no calculating | no: about a minute of float maths per tune |
| The same, more readings, fit again after refining, other side, log grid fallback | **98%** | 97% | 0.6% | 0.6 s, no calculating | no |
| Closed-form estimate (linear, unweighted) | 82% | 77% | 11% | 1.8 s | yes |
| Closed form + Levenberg-Marquardt polish | 86% | 82% | 8.2% | 1.4 s | barely |
| Closed form as a map, 3×3 log grid around each answer | 84% | 79% | 9.6% | 1.8 s | yes |
| Weighted parabola per line | 91% | 86% | 4.7% | 1.7 s (4.9) | yes |
| + bowl fit instead of single steps | 91% | 90% | 4.8% | 1.6 s (5.0) | yes |
| + log grid fallback above SWR 1.3 | 95% | 93% | 2.8% | 1.4 s* (6.6) | yes |
| + readings at each parabola's bottom | 96% | 95% | 2.2% | 1.3 s* (2.0) | yes |
| + 7×7 grid and bowl as the fallback (no log grid) | 97% | 97% | 1.9% | 0.9 s* (1.9) | yes |
| Grid and bowl alone, no model (7×7, 3 best) | 93% | 92% | 5.4% | 2.7 s* (3.5) | yes |
| **The firmware (`firmware.py`, with calculating)** | **96%** | **95%** | **2.1%** | **1.4 s* (3.2)** | **yes: 16364 of 16384 words** |

The step from 97% to the firmware's 96% is what the PIC forced: 32-bit
floats (the closed-form inverse lost 0.6%; elimination without pivoting
loses nothing), no second refining start, no extra scan when nothing
moved the SWR, a 10-reading memory. The time added is the calculating,
0.43 s per tune.

The firmware itself, in the firmware simulator (`test/run.sh`), over the
same loads, 1% noise. Times add the PIC's calculating the simulator
leaves out: 2.3 ms per reading for power and SWR in the old firmwares,
2.1 ms in the new one (the same formulas on the calculator), plus the new
search's own:

| Tuner, power | N7DDC | Log grid | Model search |
|---|---|---|---|
| as built, 10 W | 52%, SWR ≥ 2 38%, 1.4 s (3.4) | 91%, 4.1%, 3.7 s (8.2) | **96%, 2.6%, 1.5 s (3.1)** |
| as built, 5 W | 54%, 37%, 1.3 s (3.2) | 90%, 4.6%, 3.5 s (8.0) | **96%, 2.1%, 1.4 s (2.8)** |
| ideal, 10 W | 45%, 44%, 1.8 s (4.2) | 90%, 5.6%, 5.1 s (8.7) | **96%, 3.4%, 2.2 s (5.1)** |
| ideal, 5 W | 46%, 44%, 1.8 s (4.2) | 91%, 5.3%, 5.1 s (8.8) | **96%, 3.3%, 2.2 s (4.9)** |

Of the loads that start below SWR 10, the model search tunes all below
1.5 but one (log grid 98%, N7DDC 70%).

## How sure: other tuners and conditions

The lab, as built unless changed, `firmware.py` against the log grid and
DL8UG's search (below 1.5, mean time):

| Condition | Log grid | DL8UG style | Model search |
|---|---|---|---|
| 10 W | 91%, 3.3 s | 88%, 1.6 s | **96%, 1.4 s** |
| 5 W | 91%, 3.1 s | 88%, 1.6 s | **96%, 1.3 s** |
| 1.5 W | 29%, 2.8 s | 27%, 1.5 s | **61%, 1.2 s** |
| 3% detector noise | 85%, 2.9 s | 85%, 1.5 s | **95%, 1.8 s** |
| parts 10% off their values | 92%, 3.3 s | 86%, 1.6 s | **96%, 1.4 s** |
| parts 15% off | 89%, 3.3 s | 87%, 1.6 s | **94%, 1.5 s** |
| 2.5× the stray capacitance | 90%, 3.2 s | 86%, 1.6 s | **93%, 1.5 s** |

The search uses the relay values in cells 10 to 2D, so a tuner whose
measured values are entered there is calculated with its own parts.

Below about 2 W every search suffers: the reflected detector's diode does
not conduct for small mismatches, so near the match every reading says
SWR 1.0. Tune with 5 to 10 W.

## Fitting it in the PIC

- **Code**: the free XC8 stops at -O2, and every float operation on a
  PIC16 is a library call with its arguments copied byte by byte, with
  bank switching: 20 to 40 words each. Written plainly the search needed
  over 5000 words with 2460 free (after taking out N7DDC's search and the
  log grid). So the formulas are byte programs for a small calculator
  (`m_calc`): an accumulator, 16 registers, 16 constants, one byte per
  operation. The power and efficiency maths (`get_pwr`, `show_pwr`) moved
  onto it too, which saved about 1000 words, and N7DDC's `correction()`
  became a table. Plenty of smaller trims; the RAM's banks matter as
  much as the code: a nearly full RAM makes every access cost a bank
  switch.
- **Precision**: 32-bit float. Gaussian elimination of the parabola's
  normal equations needs no pivoting (the matrix is positive definite)
  and matches double precision exactly; the closed-form inverse lost 0.6%.
- **Square root**: the IEEE 754 exponent halved as a first guess (within
  6%), then three Newton steps: float precision in about 4 ms.
- **Time**, from `cycles/run.sh` (16 MHz, 4 million instructions/s): the
  fit 9.7 ms, adding a reading to a line's sums 8.9 ms, the bowl 5.4 ms,
  a relay code from a target 2.5 ms. Per tune about 0.43 s, against about
  1 s of relay waits.
- **Stack**: 15 of the 16 levels (build.sh checks).

## Sources

- DL8UG, ATU-10 NG firmware and its TUNING.md: the coarse grid on both
  sides, three non-neighbouring best points, pattern search with valley
  moves; 99.9% in his own simulation, mean 5.7 s.
- VU3ESV's tuner: an AD8302 phase detector gives Z directly; the ATU-100
  has no phase detector, hence the parabolas.
- The L-network match: any RF text; here in `model.match` and `p_lc`,
  `p_cl` in main.h.
