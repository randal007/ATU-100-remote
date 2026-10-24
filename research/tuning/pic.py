"""The trials that shaped the model search for the PIC (solve9, solve10 and
their switches): what the PIC cannot afford left out one step at a time,
and each step checked in the lab. No list of readings (the parabola fits
keep running sums), no ranking of the candidates by how well they explain
every reading (all are measured instead), a closed-form bowl fit on full
3x3 patches, only the last CACHE readings remembered (16 here; the
firmware keeps 10).

This is the exploratory version. firmware.py is the firmware's search
itself, step for step, with the programs read out of main.h."""

import math
import lab
import model
import searches

KNEE = 80 + 244
CACHE = 16
NO_AGAIN = False


def sigma_s(rho, floor):
    """How uncertain s = (1 + rho^2) / (1 - rho^2) is (model.sigma_rho, through ds/drho)"""
    sig = math.sqrt((0.012 * rho) ** 2 + (0.6 * floor / 3) ** 2 + 0.008 ** 2)
    return max(4 * rho / (1 - rho * rho) ** 2 * sig, 1e-3)


class Line:
    """Running weighted sums for s = c0 + c1 x + c2 x^2 along one line"""

    def __init__(self):
        self.m = [0.0] * 5  # sum w x^k, k = 0..4
        self.v = [0.0] * 3  # sum w s x^k, k = 0..2
        self.n = 0
        self.done = set()

    def add(self, x, s, sig):
        w = 1 / (sig * sig)
        xk = 1.0
        for k in range(5):
            self.m[k] += w * xk
            if k < 3:
                self.v[k] += w * s * xk
            xk *= x
        self.n += 1

    def fit(self):
        if self.n < 3:
            return None
        m = self.m
        a = [[m[0], m[1], m[2], self.v[0]], [m[1], m[2], m[3], self.v[1]], [m[2], m[3], m[4], self.v[2]]]
        for i in range(3):  # Gauss, partial pivoting
            p = max(range(i, 3), key=lambda k: abs(a[k][i]))
            a[i], a[p] = a[p], a[i]
            if a[i][i] == 0:
                return None
            for k in range(i + 1, 3):
                f = a[k][i] / a[i][i]
                for j in range(i, 4):
                    a[k][j] -= f * a[i][j]
        c = [0.0] * 3
        for i in (2, 1, 0):
            c[i] = (a[i][3] - sum(a[i][j] * c[j] for j in range(i + 1, 3))) / a[i][i]
        return c


class Pic:
    def __init__(self, t, l_of=model.L_OF, c_of=model.C_OF):
        self.t, self.l_of, self.c_of = t, l_of, c_of
        self.cache = []  # (l, c, sw, swr, g2)
        self.best = (10000, 0, 0, 1)
        self.lines = (Line(), Line())

    def read(self):
        """One reading: SWR x100 as get_pwr, and |gamma| corrected, and the floor"""
        f, r = self.t.raw()
        s = min(999, (f + r) * 100 // (f - r)) if r < f else 999
        cf, cr = lab.correction(f * 3), lab.correction(r * 3)
        rho = cr / cf if cf else 1.0
        return s, rho, (KNEE / cf if cf else 1.0)

    def measure(self, l, c, sw, again=True):
        for e in self.cache:
            if e[:3] == (l, c, sw):
                return e[3], e[4]
        self.t.set(l, c, sw)
        s, rho, floor = self.read()
        if s < self.best[0] and again and not NO_AGAIN:
            s2, rho2, _ = self.read()
            s, rho = (s + s2) // 2, (rho + rho2) / 2
        g2 = (rho if rho > 0 else floor / 2) ** 2
        self.cache.append((l, c, sw, s, g2))
        if len(self.cache) > CACHE:
            self.cache.pop(0)
        if s < self.best[0]:
            self.best = (s, l, c, sw)
        if l == 0 or c == 0:  # on a line: into its fit
            self.probe(l, c, rho, floor)
        return s, g2

    def probe(self, l, c, rho, floor):
        line, code = (0, l) if c == 0 else (1, c)
        if l == 0 and c == 0:
            for ln in self.lines:
                self._add(ln, 0.0, rho, floor)
            return
        ln = self.lines[line]
        if code in ln.done:
            return
        ln.done.add(code)
        x = (self.l_of if line == 0 else self.c_of)[code] * 1e-3
        self._add(ln, x, rho, floor)

    def _add(self, ln, x, rho, floor):
        r = min(rho if rho > 0 else floor / 2, 0.995)
        sig = sigma_s(r, floor) if rho > 0 else max(4 * r / (1 - r * r) ** 2 * floor / 2, 1e-3)
        ln.add(x, (1 + r * r) / (1 - r * r), sig)

    def code(self, target, of):
        return min(range(128), key=lambda b: abs(of[b] - target))

    def vertex(self, line):
        c = self.lines[line].fit()
        if not c or c[2] <= 0:
            return None
        x = -c[1] / (2 * c[2])
        if x <= 0:
            return None
        return self.code(x * 1e3, self.l_of if line == 0 else self.c_of)

    def candidates(self):
        """(R, X, w) from each line's parabola, both roots"""
        out = []
        for line in (0, 1):
            c = self.lines[line].fit()
            if not c:
                continue
            for re, im, w in model._from_parabola(c, 1):
                if line == 0:
                    z, w = complex(re * 50, im * 50), w * 50 * 1e6
                else:
                    z, w = 1 / (complex(re, im) / 50), w / 50e-3 * 1e6
                if 2 * math.pi * 0.3e6 < w < 2 * math.pi * 80e6 and z.real > 0:
                    out.append((z.real, z.imag, math.log(w / (2 * math.pi))))
        return out


def bowl(ses, start, rel=0.25, jumps=6):
    """searches.qrefine, with the closed-form fit of a full 3x3 patch"""
    local = start
    sw = start[3]
    for _ in range(jumps):
        _, l0, c0, _ = local
        sl = max(1, round(rel * l0)) if l0 else 2
        sc = max(1, round(rel * c0)) if c0 else 2
        g = {}
        for i in (-1, 0, 1):
            for j in (-1, 0, 1):
                l, c = l0 + i * sl, c0 + j * sc
                if 0 <= l <= 127 and 0 <= c <= 127:
                    v, g2 = ses.measure(l, c, sw)
                    if v < local[0]:
                        local = (v, l, c, sw)
                    g[i, j] = g2
        if len(g) == 9:
            ax = (sum(g[1, j] for j in (-1, 0, 1)) - sum(g[-1, j] for j in (-1, 0, 1))) / 6
            ay = (sum(g[i, 1] for i in (-1, 0, 1)) - sum(g[i, -1] for i in (-1, 0, 1))) / 6
            axy = (g[1, 1] - g[1, -1] - g[-1, 1] + g[-1, -1]) / 4
            axx = (sum(g[i, j] for i in (-1, 1) for j in (-1, 0, 1)) - 2 * sum(g[0, j] for j in (-1, 0, 1))) / 6
            ayy = (sum(g[i, j] for j in (-1, 1) for i in (-1, 0, 1)) - 2 * sum(g[i, 0] for i in (-1, 0, 1))) / 6
            det = 4 * axx * ayy - axy * axy
            if axx > 0 and det > 0:
                x = (-2 * ayy * ax + axy * ay) / det
                y = (-2 * axx * ay + axy * ax) / det
                x, y = max(-3, min(3, x)), max(-3, min(3, y))
                l = max(0, min(127, round(l0 + x * sl)))
                c = max(0, min(127, round(c0 + y * sc)))
                v, _ = ses.measure(l, c, sw)
                if v < local[0]:
                    local = (v, l, c, sw)
        if sl == 1 and sc == 1 and local[1:3] == (l0, c0):
            break
        if local[1:3] == (l0, c0):
            rel /= 2
        if local[0] < 105:
            break
    return local


def pattern(ses, steps=(2, 1)):
    """searches.refine around the best"""
    local = ses.best
    for step in steps:
        moved = True
        while moved:
            moved = False
            _, bl, bc, sw = local
            for dl in (-1, 0, 1):
                for dc in (-1, 0, 1):
                    l, c = bl + dl * step, bc + dc * step
                    if (dl | dc) == 0 or not (0 <= l <= 127 and 0 <= c <= 127):
                        continue
                    v, _ = ses.measure(l, c, sw)
                    if v < local[0]:
                        local = (v, l, c, sw)
                        moved = True
                        break
                if moved:
                    break
        if local[0] < 105:
            break
    return local


PROBES = [(0, 0), (4, 0), (16, 0), (64, 0), (0, 4), (0, 16), (0, 64)]
SCAN = [1, 2, 8, 32, 96, 127]
SPREAD = (0.75, 1.0, 1.33)


FB_GRID, FB_KEEP, FB_BOWL = (0, 2, 5, 12, 30, 70, 127), 3, True


def fallback_grid(ses):
    grid = FB_GRID
    pts = []
    for sw in (1, 0):
        for l in grid:
            for c in grid:
                pts.append((ses.measure(l, c, sw, again=False)[0], l, c, sw))
    pts.sort()
    for st in pts[:FB_KEEP]:
        if FB_BOWL:
            bowl(ses, st)
        else:
            big = max(1, max(st[1], st[2]) // 4)
            steps, s = [], 1
            while s <= big:
                steps.insert(0, s)
                s *= 2
            local = pattern_from(ses, st, tuple(steps))
    pattern(ses)


def pattern_from(ses, start, steps):
    keep = ses.best
    ses.best = start
    r = pattern(ses, steps)
    if keep[0] < ses.best[0]:
        ses.best = keep
    return r


def fallback_grid_old(ses, grid=(0, 3, 10, 30, 90)):
    """A coarse grid on both sides and a pattern search from the best: a
    small stand-in for the log grid"""
    for sw in (1, 0):
        for l in grid:
            for c in grid:
                ses.measure(l, c, sw, again=False)
    st = ses.best
    big = max(1, max(st[1], st[2]) // 4)
    steps, s = [], 1
    while s <= big:
        steps.insert(0, s)
        s *= 2
    pattern(ses, steps=tuple(steps))


def solve9(t, thr=130, fallback='grid', rounds=2):
    ses = Pic(t)
    sw = t.sw
    for l, c in PROBES:
        ses.measure(l, c, sw, again=False)
    if all(e[4] > 0.81 for e in ses.cache):  # nothing moved the SWR: look further
        for v in SCAN:
            ses.measure(v, 0, sw, again=False)
            ses.measure(0, v, sw, again=False)
    for _ in range(rounds):
        added = False
        for line in (0, 1):
            v = ses.vertex(line)
            if v is None:
                continue
            of = ses.l_of if line == 0 else ses.c_of
            for f in SPREAD:
                code = ses.code(of[v] * f, of)
                if code not in ses.lines[line].done:
                    ses.measure(*((code, 0) if line == 0 else (0, code)), sw, again=False)
                    added = True
        if not added:
            break
    matches = []
    for th in ses.candidates():
        for m in model.match(th):
            if m not in matches:
                matches.append(m)
    if not matches and fallback:
        fallback_grid(ses) if fallback == 'grid' else searches.log_grid(t)
        if fallback != 'grid':
            return
    scored = sorted((ses.measure(*m)[0], m) for m in matches)
    for v, m in scored[:2]:
        if v < ses.best[0] * 1.5 or v == scored[0][0]:
            bowl(ses, (v,) + m)
    pattern(ses)
    if ses.best[0] > thr and fallback:
        if fallback == 'grid':
            fallback_grid(ses)
        else:
            searches.log_grid(t)
            if t.swr100() > ses.best[0]:
                t.set(ses.best[1], ses.best[2], ses.best[3])
            return
    t.set(ses.best[1], ses.best[2], ses.best[3])


# ---- solve10: fewer float operations, for the PIC's code space ---------
#
# Along a line, the exact parabola through the three lowest readings
# (successive parabolic interpolation) instead of a weighted fit of all; it
# gives the bottom x_v, the curvature c2 and the lowest s directly, and
#     R'^2 - 2 s_min R' + 1 = 0,   w^2 = 2 R' c2,   X' = -w x_v
# The bowl can be fitted along L and C apart (no cross term).

LINE3, BOWL_SEP = True, False


class Pic3(Pic):
    def __init__(self, t):
        super().__init__(t)
        self.pts = ([], [])  # per line: (x, s)

    def _add(self, ln, x, rho, floor):
        r = min(rho if rho > 0 else floor / 2, 0.995)
        line = self.lines.index(ln)
        self.pts[line].append((x, (1 + r * r) / (1 - r * r)))
        super()._add(ln, x, rho, floor)

    def para(self, line):
        """(x_v, c2, s_min) or None"""
        if not LINE3:
            c = self.lines[line].fit()
            if not c or c[2] <= 0:
                return None
            xv = -c[1] / (2 * c[2])
            return xv, c[2], c[0] - c[1] * c[1] / (4 * c[2])
        p = sorted(self.pts[line], key=lambda q: q[1])
        # three lowest with different x
        sel = []
        for q in p:
            if all(abs(q[0] - r[0]) > 1e-9 for r in sel):
                sel.append(q)
            if len(sel) == 3:
                break
        if len(sel) < 3:
            return None
        (x1, s1), (x2, s2), (x3, s3) = sorted(sel)
        d1, d2 = (s2 - s1) / (x2 - x1), (s3 - s2) / (x3 - x2)
        c2 = (d2 - d1) / (x3 - x1)
        if c2 <= 0:
            return None
        c1 = d1 - c2 * (x1 + x2)
        xv = -c1 / (2 * c2)
        smin = s1 - c2 * (x1 - xv) ** 2
        return xv, c2, smin

    def vertex(self, line):
        p = self.para(line)
        if not p or p[0] <= 0:
            return None
        return self.code(p[0] * 1e3, self.l_of if line == 0 else self.c_of)

    def candidates(self):
        out = []
        for line in (0, 1):
            p = self.para(line)
            if not p:
                continue
            xv, c2, k = p
            k = max(k, 1.0)
            q = math.sqrt(k * k - 1)
            for re in (k + q, k - q):
                if re <= 0:
                    continue
                w = math.sqrt(2 * re * c2)
                im = -w * xv
                if line == 0:
                    z, wt = complex(re * 50, im * 50), w * 50 * 1e6
                else:
                    z, wt = 1 / (complex(re, im) / 50), w / 50e-3 * 1e6
                if 2 * math.pi * 0.3e6 < wt < 2 * math.pi * 80e6 and z.real > 0:
                    out.append((z.real, z.imag, math.log(wt / (2 * math.pi))))
        return out


def bowl_sep(ses, start, rel=0.25, jumps=6, fixed=0):
    """bowl, fitting L and C apart; fixed: steps of that many relays,
    halved, instead of a quarter of the values"""
    local = start
    sw = start[3]
    for _ in range(jumps):
        _, l0, c0, _ = local
        if fixed:
            sl = sc = fixed
        else:
            sl = max(1, round(rel * l0)) if l0 else 2
            sc = max(1, round(rel * c0)) if c0 else 2
        g = {}
        for i in (-1, 0, 1):
            for j in (-1, 0, 1):
                l, c = l0 + i * sl, c0 + j * sc
                if 0 <= l <= 127 and 0 <= c <= 127:
                    v, g2 = ses.measure(l, c, sw)
                    if v < local[0]:
                        local = (v, l, c, sw)
                    g[i, j] = g2
        if len(g) == 9:
            def axis(get):
                p, m, z = (sum(get(k, j) for j in (-1, 0, 1)) for k in (1, -1, 0))
                curv = p + m - 2 * z
                return (m - p) / (2 * curv) if curv > 0 else None
            x = axis(lambda k, j: g[k, j])
            y = axis(lambda k, j: g[j, k])
            if x is not None and y is not None:
                x, y = max(-3, min(3, x)), max(-3, min(3, y))
                l = max(0, min(127, round(l0 + x * sl)))
                c = max(0, min(127, round(c0 + y * sc)))
                v, _ = ses.measure(l, c, sw)
                if v < local[0]:
                    local = (v, l, c, sw)
        if sl == 1 and sc == 1 and local[1:3] == (l0, c0):
            break
        if local[1:3] == (l0, c0):
            rel /= 2
            fixed >>= 1
        if local[0] < 105:
            break
    return local


BOWL_PATTERN = False


def finish(ses, b):
    """The single relay steps at the end: the pattern search, or the bowl
    from the best with steps of 2 then 1"""
    if BOWL_PATTERN:
        bowl_sep(ses, ses.best, fixed=2, jumps=8)
    else:
        pattern(ses)


NO_PATTERN = NO_SCAN = False
KEEP = 2


def solve10(t, thr=130, rounds=2):
    global bowl
    b = bowl_sep if BOWL_SEP else bowl_full
    ses = Pic3(t)
    sw = t.sw
    for l, c in PROBES:
        ses.measure(l, c, sw, again=False)
    if not NO_SCAN and all(e[4] > 0.81 for e in ses.cache):
        for v in SCAN:
            ses.measure(v, 0, sw, again=False)
            ses.measure(0, v, sw, again=False)
    for _ in range(rounds):
        added = False
        for line in (0, 1):
            v = ses.vertex(line)
            if v is None:
                continue
            of = ses.l_of if line == 0 else ses.c_of
            for f in SPREAD:
                code = ses.code(of[v] * f, of)
                if code not in ses.lines[line].done:
                    ses.measure(*((code, 0) if line == 0 else (0, code)), sw, again=False)
                    added = True
        if not added:
            break
    matches = []
    for th in ses.candidates():
        for m in model.match(th):
            if m not in matches:
                matches.append(m)
    grid = not matches
    if matches:
        scored = sorted((ses.measure(*m)[0], m) for m in matches)
        for v, m in scored[:KEEP]:
            if v < ses.best[0] * 1.5 or v == scored[0][0]:
                b(ses, (v,) + m)
        if not NO_PATTERN:
            finish(ses, b)
        grid = ses.best[0] > thr
    if grid:
        pts = []
        for s in (1, 0):
            for l in FB_GRID:
                for c in FB_GRID:
                    pts.append((ses.measure(l, c, s, again=False)[0], l, c, s))
        pts.sort()
        for st in pts[:3]:
            b(ses, st)
        if not NO_PATTERN:
            finish(ses, b)
    t.set(ses.best[1], ses.best[2], ses.best[3])


bowl_full = bowl


# ---- 32-bit float and a closed-form 3x3 solve, as the PIC will do it ----

import struct


def f32(x):
    try:
        return struct.unpack('f', struct.pack('f', x))[0]
    except OverflowError:
        return math.copysign(math.inf, x)


def fit_cramer32(self):
    """Line.fit by the symmetric 3x3 inverse (cofactors), rounding every
    step to 32-bit float"""
    if self.n < 3:
        return None
    m0, m1, m2, m3, m4 = (f32(v) for v in self.m)
    v0, v1, v2 = (f32(v) for v in self.v)
    M = lambda a, b: f32(a * b)
    S = lambda a, b: f32(a - b)
    A = S(M(m2, m4), M(m3, m3))
    B = S(M(m1, m4), M(m2, m3))
    C = S(M(m1, m3), M(m2, m2))
    D = S(M(m0, m4), M(m2, m2))
    E = S(M(m0, m3), M(m1, m2))
    F = S(M(m0, m2), M(m1, m1))
    det = f32(S(M(m0, A), M(m1, B)) + M(m2, C))
    if det == 0:
        return None
    c0 = f32(f32(S(M(A, v0), M(B, v1)) + M(C, v2)) / det)
    c1 = f32(f32(S(M(D, v1), M(B, v0)) - M(E, v2)) / det)
    c2 = f32(f32(S(M(C, v0), M(E, v1)) + M(F, v2)) / det)
    return [c0, c1, c2]


def add32(self, x, s, sig):
    w = f32(1 / (sig * sig))
    xk = w
    for k in range(5):
        self.m[k] = f32(self.m[k] + xk)
        if k < 3:
            self.v[k] = f32(self.v[k] + f32(xk * s))
        xk = f32(xk * x)
    self.n += 1


def fit_ldl32(self):
    """Line.fit by elimination without pivoting (the sums make a positive
    definite matrix), unrolled as the PIC's calculator runs it, in 32-bit float"""
    if self.n < 3:
        return None
    m0, m1, m2, m3, m4 = (f32(v) for v in self.m)
    v0, v1, v2 = (f32(v) for v in self.v)
    M = lambda a, b: f32(a * b)
    S = lambda a, b: f32(a - b)
    Q = lambda a, b: f32(a / b)
    if m0 <= 0:
        return None
    l1, l2 = Q(m1, m0), Q(m2, m0)
    a11, a12, b1 = S(m2, M(l1, m1)), S(m3, M(l1, m2)), S(v1, M(l1, v0))
    a22, b2 = S(m4, M(l2, m2)), S(v2, M(l2, v0))
    if a11 <= 0:
        return None
    l3 = Q(a12, a11)
    a22, b2 = S(a22, M(l3, a12)), S(b2, M(l3, b1))
    if a22 <= 0:
        return None
    c2 = Q(b2, a22)
    c1 = Q(S(b1, M(a12, c2)), a11)
    c0 = Q(S(S(v0, M(m1, c1)), M(m2, c2)), m0)
    return [c0, c1, c2]


def grid_bowl(t, grid=(0, 2, 5, 12, 30, 70, 127), keep=3, use_bowl=True, both=True):
    """No model: the grid on both sides, then the bowl (or single steps) from
    its best points"""
    ses = Pic(t)
    pts = []
    for s in ((1, 0) if both else (t.sw,)):
        for l in grid:
            for c in grid:
                pts.append((ses.measure(l, c, s, again=False)[0], l, c, s))
    pts.sort()
    for st in pts[:keep]:
        if use_bowl:
            bowl_sep(ses, st)
        else:
            pattern_from(ses, st, (4, 2, 1))
    pattern(ses)
    t.set(ses.best[1], ses.best[2], ses.best[3])
