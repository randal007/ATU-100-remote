"""The firmware's model search (main.h tune_model), step for step, for the
lab. The calculator's programs are read out of main.h and run in 32-bit
float, as the PIC runs them, so this twin cannot drift from the firmware's
formulas; the C control flow around them is copied by hand.

    python3 firmware.py            # all loads, the summary
    python3 firmware.py 14.2 10 -400   # one load, every reading
"""

import math
import os
import re
import struct
import sys

import lab

MAIN_H = os.path.join(os.path.dirname(__file__), '../../ATU_100_EXT_board/FirmWare_PIC16F1938/main.h')


def f32(x):
    try:
        return struct.unpack('f', struct.pack('f', x))[0]
    except OverflowError:
        return math.copysign(math.inf, x)


# ---- the calculator, from main.h --------------------------------------

def _load():
    src = open(MAIN_H).read()
    defs = {}
    for name, val in re.findall(r'#define ((?:R|K)_\w+) (\d+)', src):
        defs[name] = int(val)
    ops = {'LD': 0, 'ADD': 1, 'SUB': 2, 'MUL': 3, 'DIV': 4, 'MIN': 5, 'MAX': 6, 'ST': 7}
    special = {'END': 0xF0, 'POS': 0xF1, 'SQRT': 0xF2, 'NEG': 0xF3, 'OUT': 0xF4}

    def operand(t):
        return defs[t] if t in defs else int(t)

    progs = {}
    for name, body in re.findall(r'static const char (p_\w+)\[\] = \{(.*?)\};', src, re.S):
        code = []
        for tok in re.findall(r'K\(\w+\(\w+\)\)|\w+\(\w+\)|END|POS|SQRT|NEG|OUT', body):
            m = re.fullmatch(r'K\((\w+)\((\w+)\)\)', tok)
            if m:
                code.append(0x80 | ops[m.group(1)] << 4 | operand(m.group(2)))
                continue
            m = re.fullmatch(r'(\w+)\((\w+)\)', tok)
            if m:
                code.append(ops[m.group(1)] << 4 | operand(m.group(2)))
                continue
            code.append(special[tok])
        progs[name] = code
    k = re.search(r'static const float m_k\[16\] = \{(.*?)\};', src, re.S).group(1)
    consts = [f32(eval(v.replace('1 / 1414.0', '1 / 1414.0'))) for v in k.replace('\n', ' ').split(',')]
    cache = int(re.search(r'#define M_CACHE (\d+)', src).group(1))
    knee = int(re.search(r'#define M_KNEE (\d+)', src).group(1))
    grid = [int(v) for v in re.search(r'tune_grid\[\] = \{(.*?)\}', src).group(1).split(',')]
    probe = [int(v) for v in re.search(r'm_probe\[\] = \{(.*?)\}', src).group(1).split(',')]
    return defs, progs, consts, cache, knee, grid, probe


DEFS, PROGS, KONST, M_CACHE, M_KNEE, GRID, PROBE = _load()


def m_sqrt(x):
    """main.h m_sqrt: the exponent halved, then three Newton steps"""
    bits = struct.unpack('<I', struct.pack('<f', x))[0]
    y = struct.unpack('<f', struct.pack('<I', ((bits >> 1) + 0x1FC00000) & 0xFFFFFFFF))[0]
    for _ in range(3):
        y = f32(f32(y + f32(x / y)) * 0.5)
    return y


# PIC instruction cycles, measured in gpsim (16F1788, the same core, XC8
# 3.10 -O2; research/tuning/README.md): each program, and m_nearest
CYCLES = {'p_rho': 6104, 'p_sig_read': 9088, 'p_sig_knee': 6000, 'p_add': 35504, 'p_fit': 38816,
          'p_vertex': 18504, 'p_roots': 21368, 'p_plus': 640, 'p_minus': 900, 'p_ant': 16536,
          'p_admit': 10832, 'p_check': 9408, 'p_lc_s': 11008, 'p_cl_s': 11008, 'p_neg_s': 300,
          'p_lc': 7792, 'p_cl': 6256, 'p_target': 11248, 'p_bowl': 21680, 'm_nearest': 9808}
MIPS = 4e6  # 16 MHz


class Calc:
    def __init__(self):
        self.v = [0.0] * 16
        self.out = [0, 0, 0]
        self.cycles = 0

    def run(self, name):
        self.cycles += CYCLES[name]
        a, n = 0.0, 0
        for op in PROGS[name]:
            i = op & 15
            b = KONST[i] if op & 0x80 else self.v[i]
            kind = op >> 4 & 7
            if kind == 0:
                a = b
            elif kind in (1, 2):
                a = f32(a + (b if kind == 1 else -b))
            elif kind == 3:
                a = f32(a * b)
            elif kind == 4:
                a = f32(a / b) if b else math.copysign(math.inf, a)
            elif kind == 5:
                if b < a:
                    a = b
            elif kind == 6:
                if not (b < a):
                    a = b
            elif not op & 0x80:
                self.v[i] = a
            elif i == 0:
                return True
            elif i == 1:
                if not a > 0:
                    return False
            elif i == 2:
                a = m_sqrt(a)
            elif i == 3:
                a = -a
            else:
                self.out[n] = int(a) if abs(a) < 2 ** 31 else 0
                n += 1
        raise RuntimeError('no END')


# ---- the search ---------------------------------------------------------

R = DEFS
T_BEST = 3


class Firmware:
    def __init__(self, t, ind=lab.IND_NH, cap=lab.CAP_PF, trace=False):
        self.t, self.ind, self.cap, self.trace = t, ind, cap, trace
        self.calc = Calc()
        self.l_max = self.c_max = 127
        self.top = [[0, 0, 0, 0] for _ in range(4)]  # swr, l, c, sw
        self.top[T_BEST][0] = 10000
        self.cache = []
        self.sums = [[0.0] * 8, [0.0] * 8]
        self.g = [0.0] * 9
        self.n = [0, 0]
        self.frozen = False

    @property
    def best(self):
        return self.top[T_BEST][0]

    def value(self, code, cap):
        val = self.cap if cap else self.ind
        return sum(val[i] for i in range(7) if code >> i & 1)

    def nearest(self, t, cap):
        self.calc.cycles += CYCLES['m_nearest']
        t &= 0xFFFF  # unsigned int
        val = self.cap if cap else self.ind
        mx = self.c_max if cap else self.l_max
        g = v = best = 0
        bd = 0xFFFF
        for b in range(1, 129):
            d = abs(v - t)
            if d < bd and g <= mx:
                bd, best = d, g
            if b == 128:
                return best
            i = (b & -b).bit_length() - 1
            g ^= 1 << i
            v += val[i] if g >> i & 1 else -val[i]

    def read(self):
        f, r = self.t.raw()
        s = min(999, (f + r) * 100 // (f - r)) if r < f else 999
        return s, lab.correction(f * 3), lab.correction(r * 3)

    def measure(self, l, c, sw, again):
        v = self.calc.v
        for e in self.cache:
            if e[:3] == (l, c, sw):
                v[R['R_G2']] = e[4]
                return e[3]
        self.t.set(l, c, sw)
        s, cf, cr = self.read()
        knee = M_KNEE
        if again and s < self.best:
            s2, cf2, cr2 = self.read()
            s, cf, cr, knee = (s + s2) // 2, cf + cf2, cr + cr2, 2 * M_KNEE
        if cf == 0:
            cr = cf = 1
        v[R['R_F']] = float(knee)
        v[R['R_R']] = float(cr if cr else knee >> 1)
        v[R['R_CF']] = float(cf)
        self.calc.run('p_rho')
        self.cache.append((l, c, sw, s, v[R['R_G2']]))
        if len(self.cache) > M_CACHE:
            self.cache.pop(0)
        if self.trace:
            print(f'    L={l:3d} C={c:3d} sw={sw} swr={s / 100:5.2f} |g|^2={v[R["R_G2"]]:.4f}')
        if s < self.best:
            self.top[T_BEST] = [s, l, c, sw]
        if not self.frozen:
            if c == 0:
                self.add(0, l, cr)
            if l == 0:
                self.add(1, c, cr)
        return s

    def add(self, line, code, cr):
        v = self.calc.v
        v[0:8] = self.sums[line]
        v[R['R_X']] = float(self.value(code, line))
        self.calc.run('p_sig_read' if cr else 'p_sig_knee')
        self.calc.run('p_add')
        self.sums[line] = v[0:8]
        self.n[line] += 1

    def fit(self, line):
        if self.n[line] < 3:
            return False
        self.calc.v[0:8] = self.sums[line]
        return self.calc.run('p_fit')

    def codes(self, line):
        c = self.calc
        out = []
        if not (self.fit(line) and c.run('p_roots')):
            return out
        for root in ('p_plus', 'p_minus'):
            if not (c.run(root) and c.run('p_ant')):
                continue
            if line:
                c.run('p_admit')
            if not c.run('p_check'):
                continue
            for side in (0, 1):
                if c.run('p_lc_s' if side else 'p_cl_s'):
                    for sign in (0, 1):
                        if sign:
                            c.run('p_neg_s')
                        if c.run('p_lc' if side else 'p_cl') and c.run('p_target'):
                            out.append((self.nearest(c.out[0], 0), self.nearest(c.out[1], 1), side))
        return out

    def top_add(self, n, l, c, sw, s):
        w = 0
        for p in range(n):
            if self.top[w][0] and (self.top[p][0] == 0 or self.top[p][0] > self.top[w][0]):
                w = p
        if self.top[w][0] == 0 or s < self.top[w][0]:
            self.top[w] = [s, l, c, sw]

    @staticmethod
    def step(v, dv, fixed):
        if fixed:
            return fixed
        if v == 0:
            return 2
        v = ((v + dv // 2) & 0xFF) // dv
        return v or 1

    @staticmethod
    def move(v0, out, step, mx):
        v = v0 + ((out * step + 8) >> 4)
        return 0 if v < 0 else mx if v > mx else v

    def bowl(self, n, fixed):
        lv, ll, lc, sw = self.top[n]
        dv = 4
        jump = 0
        while jump < 6 + fixed:
            l0, c0 = ll, lc
            sl, sc = self.step(l0, dv, fixed), self.step(c0, dv, fixed)
            l, c = l0 - sl, c0 - 2 * sc
            k = 0
            for i in range(10):
                if i < 9:
                    c += sc
                    if i in (3, 6):
                        c = c0 - sc
                        l += sl
                    if l < 0 or c < 0 or l > self.l_max or c > self.c_max:
                        continue
                else:
                    self.calc.v[0:9] = self.g
                    if k < 9 or not self.calc.run('p_bowl'):
                        break
                    l = self.move(l0, self.calc.out[0], sl, self.l_max)
                    c = self.move(c0, self.calc.out[1], sc, self.c_max)
                s = self.measure(l, c, sw, True)
                if s < lv:
                    lv, ll, lc = s, l, c
                if i < 9:
                    self.g[i] = self.calc.v[R['R_G2']]
                    k += 1
            if ll == l0 and lc == c0:
                if sl == 1 and sc == 1:
                    break
                dv = (dv << 1) & 0xFF
                fixed >>= 1
            if lv < 105:
                break
            jump += 1

    def line(self, line, code, sw):
        return self.measure(0 if line else code, code if line else 0, sw, False)

    def grid(self):
        self.frozen = True
        for i in range(3):
            self.top[i] = [0, 0, 0, 0]
        for sw in (1, 0):
            for l in GRID:
                if l > self.l_max:
                    break
                for c in GRID:
                    if c > self.c_max:
                        break
                    s = self.measure(l, c, sw, False)
                    self.top_add(3, l, c, sw, s)
        for i in range(3):
            if self.top[i][0]:
                self.bowl(i, 0)
        self.bowl(T_BEST, 2)

    def search(self):
        sw = self.t.sw
        for i in range(7):
            self.line(i > 3, PROBE[i], sw)
        for _ in range(2):
            added = False
            for line in (0, 1):
                if not (self.fit(line) and self.calc.run('p_vertex')):
                    continue
                n = self.n[line]
                targets = list(self.calc.out)
                for i in range(3):
                    self.line(line, self.nearest(targets[i], line), sw)
                if self.n[line] != n:
                    added = True
            if not added:
                break
        self.frozen = True
        self.top[0][0] = 0
        for line in (0, 1):
            for l, c, s in self.codes(line):
                v = self.measure(l, c, s, True)
                self.top_add(1, l, c, s, v)
        if self.top[0][0] == 0:
            return 2
        self.bowl(0, 0)
        self.bowl(T_BEST, 2)
        return 2 if self.best > 130 else 1

    def tune(self):
        if self.search() == 2:
            if self.trace:
                print('  grid')
            self.grid()
        _, l, c, sw = self.top[T_BEST]
        self.t.set(l, c, sw)


class Tuner(lab.Tuner):
    """lab.Tuner with the firmware's relay waits: one for the capacitor side,
    one for L and C together"""

    def set(self, l, c, sw=None):
        if sw is None:
            sw = self.sw
        self.time += lab.T_RELAY * ((sw != self.sw) + (l != self.l or c != self.c))
        self.l, self.c, self.sw = l, c, sw


def tune(t):
    """Tune; the PIC's calculating time goes onto t.time, and is returned"""
    f = Firmware(t)
    f.tune()
    secs = f.calc.cycles / MIPS
    t.time += secs
    return secs


def run(noise=0.01, real=True, fwd_mv=900):
    rows = []
    for i, (mhz, r, x, best) in enumerate(lab.loads()):
        t = Tuner(complex(r, x), mhz * 1e6, real=real, noise=noise, seed=1 + i, fwd_mv=fwd_mv)
        calc = tune(t)
        rows.append((t.true_swr(), best, t.time, t.n, calc))
    n = len(rows)
    return {
        'loads': n,
        'below_1.5': sum(s < 1.5 for s, *_ in rows) / n,
        'near_best': sum(s <= max(b + 0.1, 1.25) for s, b, *_ in rows) / n,
        'swr2_or_worse': sum(s >= 2 for s, *_ in rows) / n,
        'mean_time': sum(r[2] for r in rows) / n,
        'p90_time': sorted(r[2] for r in rows)[int(0.9 * n)],
        'mean_measurements': sum(r[3] for r in rows) / n,
        'mean_calc': sum(r[4] for r in rows) / n,
        'p90_calc': sorted(r[4] for r in rows)[int(0.9 * n)],
    }


if __name__ == '__main__':
    if len(sys.argv) == 4:
        mhz, r, x = (float(a) for a in sys.argv[1:])
        t = Tuner(complex(r, x), mhz * 1e6)
        f = Firmware(t, trace=True)
        f.tune()
        print(f'  tuned SWR {t.true_swr():.2f} in {t.time + f.calc.cycles / MIPS:.2f} s '
              f'({f.calc.cycles / MIPS:.2f} s calculating), {t.n} readings')
    else:
        res = run()
        lab.show('firmware (main.h)', res)
        print(f'  of that, calculating on the PIC: {res["mean_calc"]:.2f} s (90% {res["p90_calc"]:.2f})')
