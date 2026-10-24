"""Working out the antenna from a few measurements, and the match from the
antenna: the maths behind the "measure, calculate, verify" searches.

The tuner measures only |gamma| (forward and reflected strength), never the
phase, and does not know the frequency. But |gamma| at a few known relay
settings is enough: three unknowns (the antenna's R and X, and the
frequency) and more measurements than that. With the antenna known, the
L network has an exact solution on each capacitor side.
"""

import cmath
import math

IND_NH = [50, 110, 220, 450, 1000, 2200, 4500]
CAP_PF = [10, 22, 47, 100, 220, 470, 1000]


def value(bits, values):
    return sum(v for i, v in enumerate(values) if bits >> i & 1)


# Every relay setting's total, nominal: code -> nH or pF
L_OF = [value(b, IND_NH) for b in range(128)]
C_OF = [value(b, CAP_PF) for b in range(128)]


def gamma_ideal(z, f, l_bits, c_bits, c_ant):
    """The network as the firmware knows it: nominal values, no strays"""
    w = 2 * math.pi * f
    zl = 1j * w * L_OF[l_bits] * 1e-9
    yc = 1j * w * C_OF[c_bits] * 1e-12
    zin = zl + 1 / (1 / z + yc) if c_ant else 1 / (1 / (zl + z) + yc)
    return (zin - 50) / (zin + 50)


def cost(theta, probes):
    """How badly an antenna guess explains the measurements. probes:
    (l, c, sw, rho, floor): floor is the smallest |gamma| the reflected
    detector can see; a reading of 0 only says |gamma| was below it"""
    r, x, lf = theta
    if r <= 0.5 or lf < math.log(1.0e6) or lf > math.log(40e6):
        return 1e9
    z, f = complex(r, x), math.exp(lf)
    e = 0.0
    for l, c, sw, rho, floor in probes:
        g = abs(gamma_ideal(z, f, l, c, sw))
        if rho <= 0:  # below the knee: only too high is wrong
            e += max(0.0, g - floor) ** 2
        else:
            e += (g - rho) ** 2
    return e


def nelder_mead(fn, x0, steps, iters=200):
    pts = [list(x0)]
    for i, s in enumerate(steps):
        p = list(x0)
        p[i] += s
        pts.append(p)
    vals = [fn(p) for p in pts]
    for _ in range(iters):
        order = sorted(range(len(pts)), key=lambda i: vals[i])
        pts = [pts[i] for i in order]
        vals = [vals[i] for i in order]
        n = len(x0)
        centre = [sum(p[j] for p in pts[:-1]) / n for j in range(n)]
        refl = [centre[j] + (centre[j] - pts[-1][j]) for j in range(n)]
        fr = fn(refl)
        if fr < vals[0]:
            exp_ = [centre[j] + 2 * (centre[j] - pts[-1][j]) for j in range(n)]
            fe = fn(exp_)
            pts[-1], vals[-1] = (exp_, fe) if fe < fr else (refl, fr)
        elif fr < vals[-2]:
            pts[-1], vals[-1] = refl, fr
        else:
            con = [centre[j] + 0.5 * (pts[-1][j] - centre[j]) for j in range(n)]
            fc = fn(con)
            if fc < vals[-1]:
                pts[-1], vals[-1] = con, fc
            else:
                for i in range(1, len(pts)):
                    pts[i] = [pts[0][j] + 0.5 * (pts[i][j] - pts[0][j]) for j in range(n)]
                    vals[i] = fn(pts[i])
    i = min(range(len(pts)), key=lambda i: vals[i])
    return pts[i], vals[i]


def fit(probes, starts=None):
    """The antenna (R, X, f) that best explains the probes: a coarse look over
    the bypass |gamma| circle and the frequency, then Nelder-Mead from the
    best few starting points"""
    bypass = next((p for p in probes if p[0] == 0 and p[1] == 0), None)
    rho0 = min(max(bypass[3], 0.02), 0.97) if bypass else 0.5
    cands = []
    for k in range(24):  # where on the |gamma| = rho0 circle
        g0 = rho0 * cmath.exp(1j * 2 * math.pi * k / 24)
        z = 50 * (1 + g0) / (1 - g0)
        if z.real <= 0.5:
            continue
        for m in range(16):  # 1.6 to 32 MHz, log spaced
            lf = math.log(1.6e6) + m / 15 * math.log(20)
            th = (z.real, z.imag, lf)
            cands.append((cost(th, probes), th))
    if starts:
        cands += [(cost(s, probes), s) for s in starts]
    cands.sort(key=lambda c: c[0])
    best = None
    for _, th in cands[:4]:
        p, v = nelder_mead(lambda q: cost(q, probes), th, [max(th[0] * 0.2, 2), max(abs(th[1]) * 0.2, 5), 0.15])
        if best is None or v < best[1]:
            best = (p, v)
    return best  # ((R, X, log f), cost)


def nearest_code(target, of):
    return min(range(128), key=lambda b: abs(of[b] - target))


def match(theta):
    """Exact L-network solutions for the antenna theta, rounded to relay
    settings: [(l_bits, c_bits, sw)] (up to 2 per capacitor side)"""
    r, x, lf = theta
    z, w = complex(r, x), 2 * math.pi * math.exp(lf)
    out = []
    # C at the antenna side: shunt C across the antenna, then series L
    y = 1 / z
    g, b = y.real, y.imag
    if 0 < g < 1 / 50:
        root = math.sqrt(g / 50 - g * g)
        for s in (root, -root):
            wc = s - b
            if wc < 0:
                continue
            zp = 1 / complex(g, s)
            wl = -zp.imag
            if wl < 0:
                continue
            out.append((nearest_code(wl / w * 1e9, L_OF), nearest_code(wc / w * 1e12, C_OF), 1))
    # C at the transmitter side: series L, then shunt C at the input
    if 0 < r < 50:
        root = math.sqrt(50 * r - r * r)
        for s in (root, -root):
            wl = s - x
            if wl < 0:
                continue
            zs = complex(r, x + wl)
            wc = -(1 / zs).imag
            if wc < 0:
                continue
            out.append((nearest_code(wl / w * 1e9, L_OF), nearest_code(wc / w * 1e12, C_OF), 0))
    return out


# ---- A closed-form estimate the PIC can afford -----------------------
#
# A probe that adds only a series L (C = 0) changes the antenna Z = R + jX
# to Z + jwL. Its |gamma| puts (R, X + wL) on a circle:
#     (R - 50 s)^2 + (X + wL)^2 = 2500 (s^2 - 1),   s = (1 + |g|^2) / (1 - |g|^2)
# Subtracting the bypass probe's circle (L = 0) leaves, for each probe k,
#     -100 (s_k - s_0) R + 2 L_k (w X) + L_k^2 (w^2) = 0
# which is linear in (R, wX, w^2). Probes that add only a shunt C do the same
# for the admittance G + jB:  -(s_k - s_0) G / 25 + 2 C_k (w B) + C_k^2 (w^2) = 0
# (in units where 1/50 S is 1). Two or more probes of a kind fix the ratios;
# the bypass circle then fixes the scale (a quadratic).


def _s(rho):
    rho = min(rho, 0.995)
    return (1 + rho * rho) / (1 - rho * rho)


def _null(rows):
    """The direction v with rows . v as close to 0 as can be (least squares,
    3 unknowns): the smallest eigenvector of A^T A, by inverse iteration.
    Columns are scaled to equal size first (and back after)."""
    k = [math.sqrt(sum(r[i] * r[i] for r in rows)) or 1.0 for i in range(3)]
    rows = [[r[i] / k[i] for i in range(3)] for r in rows]
    m = [[sum(r[i] * r[j] for r in rows) for j in range(3)] for i in range(3)]
    tr = m[0][0] + m[1][1] + m[2][2]
    for i in range(3):
        m[i][i] += 1e-9 * tr  # keeps it invertible
    v = [1.0, 1.0, 1.0]
    for _ in range(30):
        # solve m x = v (3x3, Cramer)
        def det(a):
            return (a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
                    + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]))
        d = det(m)
        if d == 0:
            return None
        x = []
        for c in range(3):
            a = [row[:] for row in m]
            for r in range(3):
                a[r][c] = v[r]
            x.append(det(a) / d)
        n = math.sqrt(sum(t * t for t in x)) or 1
        v = [t / n for t in x]
    return [v[i] / k[i] for i in range(3)]


def estimate(probes):
    """(R, X, log f) from the L-only and C-only probes, closed form; None if
    they do not pin it down. probes: (l, c, sw, rho, floor)"""
    by = next((p for p in probes if p[0] == 0 and p[1] == 0), None)
    if by is None or by[3] <= 0:
        return None
    s0 = _s(by[3])
    cands = []
    # series L probes: unknowns (R, wX, w^2); L in henries
    # units: L in uH and w in rad/us, so wL is in ohms
    rows = [(-100 * (_s(p[3]) - s0), 2 * L_OF[p[0]] * 1e-3, (L_OF[p[0]] * 1e-3) ** 2)
            for p in probes if p[1] == 0 and p[0] and p[3] > 0]
    if len(rows) >= 2:
        v = _null(rows)
        if v and v[2]:
            if v[2] < 0:
                v = [-t for t in v]
            r1, p1, q1 = v  # (R, wX, w^2) = lam (r1, p1, q1), lam > 0
            # bypass circle: R^2 + X^2 - 100 R s0 + 2500 = 0, X^2 = lam p1^2 / q1
            a, b, c = r1 * r1, p1 * p1 / q1 - 100 * r1 * s0, 2500
            disc = b * b - 4 * a * c
            if a and disc >= 0:
                for lam in ((-b + math.sqrt(disc)) / (2 * a), (-b - math.sqrt(disc)) / (2 * a)):
                    if lam > 0 and r1 * lam > 0:
                        w = math.sqrt(lam * q1)  # rad/us
                        cands.append((r1 * lam, lam * p1 / w, w * 1e6))
    # shunt C probes: admittance in units of 1/50 S: g = 50 G, b = 50 B;
    # unknowns (g, w b', w^2) with C scaled by 50
    # units: C in nF and w in rad/us: w C is in mS, 0.05 w C in units of 1/50 S
    rows = [(-(_s(p[3]) - s0), 2 * C_OF[p[1]] * 1e-3 * 0.05, (C_OF[p[1]] * 1e-3 * 0.05) ** 2)
            for p in probes if p[0] == 0 and p[1] and p[3] > 0]
    if len(rows) >= 2:
        v = _null(rows)
        if v and v[2]:
            if v[2] < 0:
                v = [-t for t in v]
            g1, p1, q1 = v
            a, b, c = g1 * g1, p1 * p1 / q1 - 2 * g1 * s0, 1
            disc = b * b - 4 * a * c
            if a and disc >= 0:
                for lam in ((-b + math.sqrt(disc)) / (2 * a), (-b - math.sqrt(disc)) / (2 * a)):
                    if lam > 0 and g1 * lam > 0:
                        w = math.sqrt(lam * q1)  # rad/us
                        y = complex(g1 * lam, lam * p1 / w) / 50
                        z = 1 / y
                        cands.append((z.real, z.imag, w * 1e6))
    if not cands:
        return None
    # the candidate that explains all the probes best
    best = None
    for r, x, w in cands:
        if not (0.3e6 < w / (2 * math.pi) < 60e6):
            continue
        th = (r, x, math.log(w / (2 * math.pi)))
        e = cost(th, probes)
        if best is None or e < best[1]:
            best = (th, e)
    return best


# ---- Weighted parabola fits: the robust closed form -----------------
#
# Along the series-L line (C = 0), s = (1 + |g|^2) / (1 - |g|^2) is exactly
#     s(L) = c0 + c1 L + c2 L^2,  c2 = w^2 / (100 R),  c1 = 2 X w / (100 R),
#     c0 = (R^2 + X^2 + 2500) / (100 R)
# and along the shunt-C line the same holds for the admittance (in units of
# 1/50 S, with C scaled). A weighted least-squares parabola through the
# probes gives c0, c1, c2; then R from K = c0 - c1^2 / (4 c2):
#     R^2 - 100 K R + 2500 = 0   (two roots, R and 2500 / R)
# Each probe is weighted by how well its reading fixes s: s grows without
# bound as |g| -> 1, so a reading near total mismatch counts for little.


def sigma_rho(rho, floor):
    """How uncertain a |gamma| reading is: 1% noise, ADC steps and the
    diode's drop not matching the table"""
    return math.sqrt((0.012 * rho) ** 2 + (0.6 * floor / 3) ** 2 + 0.008 ** 2)


def _parabola(points):
    """Weighted least squares s = c0 + c1 x + c2 x^2; points (x, s, sigma)"""
    m = [[0.0] * 3 for _ in range(3)]
    v = [0.0] * 3
    for x, s, sig in points:
        w = 1 / (sig * sig)
        p = (1.0, x, x * x)
        for i in range(3):
            v[i] += w * p[i] * s
            for j in range(3):
                m[i][j] += w * p[i] * p[j]
    def det(a):
        return (a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
                + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]))
    d = det(m)
    if not d:
        return None
    out = []
    for c in range(3):
        a = [row[:] for row in m]
        for r in range(3):
            a[r][c] = v[r]
        out.append(det(a) / d)
    return out


def _from_parabola(c, scale):
    """(re, im, w) candidates from parabola coefficients; scale: 50 for the
    L line (ohms), 1 for the C line (units of 1/50 S, re-scaled after)"""
    c0, c1, c2 = c
    if c2 <= 0:
        return []
    k = c0 - c1 * c1 / (4 * c2)
    if k < 1:
        k = 1.0  # at a perfect match K = 1; noise can push it below
    out = []
    for re in (k + math.sqrt(k * k - 1), k - math.sqrt(k * k - 1)):
        if re <= 0:
            continue
        b = 1 / (2 * re)          # 1/(100 R) in units where 50 ohm is 1
        w = math.sqrt(c2 / b)
        im = c1 / (2 * w * b)
        out.append((re, im, w))
    return out


def estimate2(probes, all_candidates=False):
    """(R, X, log f) from weighted parabola fits along the L line and the C
    line; the candidate that best explains all probes"""
    by = next((p for p in probes if p[0] == 0 and p[1] == 0), None)
    if by is None:
        return None

    def point(p, x):
        rho = p[3] if p[3] > 0 else p[4] / 2
        rho = min(rho, 0.995)
        sig = sigma_rho(rho, p[4]) if p[3] > 0 else p[4] / 2
        s = (1 + rho * rho) / (1 - rho * rho)
        ds = 4 * rho / (1 - rho * rho) ** 2 * sig
        return (x, s, max(ds, 1e-3))

    cands = []
    # L line, x = L in uH, w in rad/us, z normalised to 50 ohm: s = c0 + c1 x + c2 x^2
    # with the quantities in ohm/50: R' = R/50, X' = X/50, w' = w/50
    lp = [point(by, 0.0)] + [point(p, L_OF[p[0]] * 1e-3) for p in probes if p[1] == 0 and p[0]]
    if len(lp) >= 3:
        c = _parabola(lp)
        if c:
            for re, im, w in _from_parabola(c, 50):
                cands.append((complex(re * 50, im * 50), w * 50 * 1e6))
    cp = [point(by, 0.0)] + [point(p, C_OF[p[1]] * 1e-3) for p in probes if p[0] == 0 and p[1]]
    if len(cp) >= 3:
        c = _parabola(cp)
        if c:
            for re, im, w in _from_parabola(c, 1):
                # admittance y = (re + j im) / 50 S; w C: C in nF, w' in rad/us * 50e-3
                y = complex(re, im) / 50
                cands.append((1 / y, w / 50e-3 * 1e6))
    out = []
    for z, w in cands:
        f = w / (2 * math.pi)
        if not (0.3e6 < f < 80e6) or z.real <= 0:
            continue
        th = (z.real, z.imag, math.log(f))
        out.append((th, cost(th, probes)))
    out.sort(key=lambda o: o[1])
    if all_candidates:
        return out
    return out[0] if out else None
