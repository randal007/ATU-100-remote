"""Tuning research lab: the ATU-100 EXT relay network, the antenna and the
detector as the simulator models them (test/host.c, REAL=1: parts a few %
off, strays, coil loss, diodes with more drop than N7DDC's table), the
firmware's way of measuring, and a cost model for time. Search algorithms
are tried here first; the good ones go into the firmware and are measured
again in the simulator proper.

Python 3, no packages. See README.md in this folder.
"""

import cmath
import csv
import math
import os
import random

IND_NH = [50, 110, 220, 450, 1000, 2200, 4500]  # as the firmware believes
CAP_PF = [10, 22, 47, 100, 220, 470, 1000]
TOL_L = [+0.06, -0.04, +0.03, -0.07, +0.05, -0.02, +0.04]  # as built (unknown to the firmware)
TOL_C = [-0.05, +0.07, -0.03, +0.04, -0.06, +0.02, -0.04]
C_STRAY_TX, C_STRAY_ANT, L_WIRING, COIL_Q = 15e-12, 12e-12, 40e-9, 80.0

# Time on the PIC: each relay group changed waits the settling time (cell 03,
# 15 ms, about 15% longer with the serial interrupt); a measurement itself
# takes about 1 ms
T_RELAY, T_MEASURE = 0.0145, 0.001  # calibrated against the firmware in the simulator


def nominal(bits, values):
    return sum(v for i, v in enumerate(values) if bits >> i & 1)


def gamma(z_load, f, l_bits, c_bits, c_ant, real=True):
    """Reflection coefficient at the tuner's input"""
    w = 2 * math.pi * f
    l = sum(IND_NH[i] * 1e-9 * (1 + (TOL_L[i] if real else 0)) for i in range(7) if l_bits >> i & 1)
    c = sum(CAP_PF[i] * 1e-12 * (1 + (TOL_C[i] if real else 0)) for i in range(7) if c_bits >> i & 1)
    z = z_load
    if real:
        l += L_WIRING
        z = 1 / (1 / z + 1j * w * C_STRAY_ANT)
    zl = 1j * w * l + (w * l / COIL_Q if real else 0)
    yc = 1j * w * c
    zin = zl + 1 / (1 / z + yc) if c_ant else 1 / (1 / (zl + z) + yc)
    if real:
        zin = 1 / (1 / zin + 1j * w * C_STRAY_TX)
    return (zin - 50) / (zin + 50)


def swr(g):
    a = abs(g)
    return 99.0 if a >= 0.98 else (1 + a) / (1 - a)


# The detector, as host.c: N7DDC's correction table, and diodes with a 15% bigger drop
_UPTO = [171, 328, 582, 820, 1100, 2181, 3322, 4623, 5862, 7146, 8502, 10500]
_ADD = [244, 254, 280, 297, 310, 430, 484, 530, 648, 743, 800, 840]


def drop(d):
    for u, a in zip(_UPTO, _ADD):
        if d <= u:
            return a
    return 860


def correction(x):
    """N7DDC's correction(), as the firmware does it ("x3" millivolts)"""
    return 0 if x <= 80 else x + drop(x)


def diode(v, real=True):
    if not real:
        return v
    if v <= 80 + 1.15 * 244:
        return 0.0
    lo, hi = 80.0, v
    for _ in range(40):
        d = (lo + hi) / 2
        if d + 1.15 * drop(d) > v:
            hi = d
        else:
            lo = d
    return lo


def adc(mv):
    """get_forward / get_reverse: 10-bit ADC, 1.024, 2.048 or 4.096 V range"""
    for ref, scale in ((1024, 1), (2048, 2), (4096, 4)):
        counts = min(1023, int(mv * 1023 / ref))
        if counts <= 1000 or scale == 4:
            return counts * scale
    return 4092


class Tuner:
    """One tune on one antenna load: relays, measurements, the time they take"""

    def __init__(self, z_load, f, real=True, noise=0.01, fwd_mv=900, seed=1):
        self.z, self.f, self.real, self.noise, self.fwd_mv = z_load, f, real, noise, fwd_mv
        self.rng = random.Random(seed)
        self.l = self.c = 0
        self.sw = 1
        self.time = 0.0
        self.n = 0  # measurements

    def set(self, l, c, sw=None):
        if sw is None:
            sw = self.sw
        groups = (l != self.l) + (c != self.c) + (sw != self.sw)
        self.time += groups * T_RELAY
        self.l, self.c, self.sw = l, c, sw

    def _noisy(self, mv):
        return mv * (1 + self.rng.gauss(0, self.noise)) if self.noise else mv

    def raw(self):
        """Forward and reverse ADC values (in the firmware's units), one reading"""
        self.n += 1
        self.time += T_MEASURE
        g = abs(gamma(self.z, self.f, self.l, self.c, self.sw, self.real))
        fwd = self.fwd_mv
        if self.real:
            vf = 3 * fwd + 1.15 * drop(3 * fwd)
            rev = diode(vf * g) / 3
        else:
            rev = fwd * g
        return adc(self._noisy(fwd)), adc(self._noisy(rev))

    def swr100(self):
        """g_i_SWR as get_pwr() computes it: from the raw readings, 100..999"""
        f, r = self.raw()
        if r >= f:
            return 999
        s = (f + r) * 100 // (f - r)
        return min(s, 999)

    def rho(self):
        """|gamma| from both readings corrected (for model fitting), and the readings"""
        f, r = self.raw()
        cf, cr = correction(f * 3), correction(r * 3)
        return (cr / cf if cf else 1.0), f, r

    def true_swr(self):
        return swr(gamma(self.z, self.f, self.l, self.c, self.sw, self.real))


def loads(path=None):
    """The simulator sweep's loads: (MHz, R, X, best SWR) for the matchable ones"""
    path = path or os.path.join(os.path.dirname(__file__), 'loads_real.csv')
    with open(path) as f:
        return [(float(r['MHz']), float(r['R']), float(r['X']), float(r['best_swr'])) for r in csv.DictReader(f)]


def run(algorithm, real=True, noise=0.01, limit=None, seed=1):
    """Tune every load with algorithm(tuner) and summarise"""
    rows = []
    for i, (mhz, r, x, best) in enumerate(loads()[:limit]):
        t = Tuner(complex(r, x), mhz * 1e6, real=real, noise=noise, seed=seed + i)
        algorithm(t)
        rows.append((t.true_swr(), best, t.time, t.n))
    n = len(rows)
    return {
        'loads': n,
        'below_1.5': sum(s < 1.5 for s, *_ in rows) / n,
        'near_best': sum(s <= max(b + 0.1, 1.25) for s, b, *_ in rows) / n,
        'swr2_or_worse': sum(s >= 2 for s, *_ in rows) / n,
        'mean_time': sum(t for *_, t, _ in rows) / n,
        'p90_time': sorted(t for *_, t, _ in rows)[int(0.9 * n)],
        'mean_measurements': sum(m for *_, m in rows) / n,
    }


def show(name, res):
    print(f"{name:28} below 1.5 {100*res['below_1.5']:5.1f}%  near best {100*res['near_best']:5.1f}%  "
          f"SWR>=2 {100*res['swr2_or_worse']:4.1f}%  time {res['mean_time']:.2f} s (90% {res['p90_time']:.2f})  "
          f"{res['mean_measurements']:.0f} meas")
