"""Search algorithms for lab.py. Each takes a lab.Tuner and leaves the relays
on its answer; the lab measures the true SWR there and the time taken."""

GRID = [0, 1, 2, 3, 4, 6, 8, 11, 16, 22, 32, 45, 64, 90, 127]


def log_grid(t, double_check=True):
    """The firmware's log grid search (main.h tune_log), as it is now"""
    best = {'swr': 10000, 'l': 0, 'c': 0, 'sw': 1}

    def t_try(l, c, sw=None):
        t.set(l, c, sw)
        s = t.swr100()
        if s < best['swr'] and double_check:
            s = (s + t.swr100()) // 2
        if s < best['swr']:
            best.update(swr=s, l=l, c=c, sw=t.sw)
        return s

    sw = t.sw
    for _ in range(2):
        sw = 1 - sw
        last_row, row_rises = 10000, 0
        for li in GRID:
            row, rises = 10000, 0
            for ci in GRID:
                s = t_try(li, ci, sw)
                if s < row:
                    row, rises = s, 0
                elif s > row + row // 20:
                    rises += 1
                    if rises == 2:
                        break
            if best['swr'] < 110:
                break
            if row > last_row + last_row // 20 and best['swr'] < 999:
                row_rises += 1
                if row_rises == 2:
                    break
            else:
                row_rises = 0
            last_row = row
        if best['swr'] <= 130:
            break
    step = 4
    while step:
        moved = False
        for dl in (-1, 0, 1):
            for dc in (-1, 0, 1):
                l, c = best['l'] + dl * step, best['c'] + dc * step
                if (dl | dc) == 0 or not (0 <= l <= 127 and 0 <= c <= 127):
                    continue
                before = (best['l'], best['c'])
                t_try(l, c, best['sw'])
                if (best['l'], best['c']) != before:
                    moved = True
        if not moved:
            step >>= 1
        if best['swr'] < 105:
            break
    t.set(best['l'], best['c'], best['sw'])


# ---- "Measure, calculate, verify" -------------------------------------

import lab
import model

PROBES = [(0, 0, 1), (4, 0, 1), (16, 0, 1), (64, 0, 1), (0, 4, 1), (0, 16, 1), (0, 64, 1)]
KNEE = 80 + 244  # below this the reflected diode gives 0 (N7DDC's table)


class Session:
    """Measurements of one tune: each as the firmware's SWR (x100) and as a
    probe for the model fit, plus the best setting so far"""

    def __init__(self, t):
        self.t, self.probes, self.seen = t, [], {}
        self.best = (10000, 0, 0, 1)

    def measure(self, l, c, sw, again=True):
        key = (l, c, sw)
        if key in self.seen:
            return self.seen[key]
        self.t.set(l, c, sw)
        rho, f, r = self.t.rho()
        s = min(999, (f + r) * 100 // (f - r)) if r < f else 999
        if s < self.best[0] and again:  # a new best: once more, as the firmware does
            rho2, f2, r2 = self.t.rho()
            s2 = min(999, (f2 + r2) * 100 // (f2 - r2)) if r2 < f2 else 999
            s, rho = (s + s2) // 2, (rho + rho2) / 2
        cf = lab.correction(f * 3)
        self.probes.append((l, c, sw, rho, KNEE / cf if cf else 1.0))
        self.seen[key] = s
        if s < self.best[0]:
            self.best = (s, l, c, sw)
        return s


def refine(ses, steps=(4, 2, 1), start=None):
    """Pattern search around the best (or from start), as the log grid's
    last part; returns the best point it reached"""
    local = start or ses.best
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
                    v = ses.measure(l, c, sw)
                    if v < local[0]:
                        local = (v, l, c, sw)
                        moved = True
                        break
                if moved:
                    break
        if local[0] < 105:
            break
    return local


def solve(t, rounds=3, fallback=False):
    ses = Session(t)
    for l, c, sw in PROBES:
        ses.measure(l, c, sw, again=False)
    theta, starts = None, None
    for _ in range(rounds):
        theta, _ = model.fit(ses.probes, starts)
        starts = [theta]
        new = [m for m in model.match(theta) if m not in ses.seen]
        if not new:
            break
        for m in new:
            ses.measure(*m)
        if ses.best[0] < 110:
            break
    refine(ses)
    if fallback and ses.best[0] > 150:
        log_grid(t)
        return
    t.set(ses.best[1], ses.best[2], ses.best[3])


SCAN = [1, 2, 8, 32, 96, 127]


def solve2(t, fallback=True):
    """solve, plus: extra probes when the first seven say nothing, a fresh
    calculation after refining (the readings near the match tell the most),
    the other capacitor side when it is close, and the log grid as a last
    resort above SWR 3"""
    ses = Session(t)
    for p in PROBES:
        ses.measure(*p, again=False)
    if min(p[3] for p in ses.probes) > 0.9:  # nothing moved the SWR: look further
        for v in SCAN:
            ses.measure(v, 0, 1, again=False)
            ses.measure(0, v, 1, again=False)
    theta = None
    for cycle in range(3):
        for _ in range(3):
            theta, _ = model.fit(ses.probes, [theta] if theta else None)
            new = [m for m in model.match(theta) if m not in ses.seen]
            if not new:
                break
            for m in new:
                ses.measure(*m)
        before = ses.best
        refine(ses)
        if ses.best[0] < 110 or ses.best == before and cycle:
            break
    # the other capacitor side, if the calculation puts a match there too
    _, bl, bc, bsw = ses.best
    for m in model.match(theta):
        if m[2] != bsw:
            v = ses.measure(*m)
            if v < ses.best[0] * 1.3:
                refine(ses, start=(v,) + m)
    if fallback and ses.best[0] > 300:
        log_grid(t)
        return
    t.set(ses.best[1], ses.best[2], ses.best[3])


def solve3(t, fallback=True, other_side=True):
    """solve2 with the closed-form estimate (what the PIC can afford): no
    general fitting. Extra probes when the first seven say nothing; the
    estimate's match on each side, then refine; the log grid above SWR 3"""
    ses = Session(t)
    for p in PROBES:
        ses.measure(*p, again=False)
    est = model.estimate(ses.probes)
    if est is None or min(p[3] for p in ses.probes) > 0.9:
        for v in SCAN:
            ses.measure(v, 0, 1, again=False)
            ses.measure(0, v, 1, again=False)
        est = model.estimate(ses.probes) or est
    cands = model.match(est[0]) if est else []
    for m in cands:
        ses.measure(*m)
    refine(ses)
    if other_side:
        bsw = ses.best[3]
        for m in cands:
            if m[2] != bsw and ses.seen.get(m, 999) < ses.best[0] * 1.3:
                refine(ses, start=(ses.seen[m],) + m)
    if fallback and ses.best[0] > 300:
        log_grid(t)
        return
    t.set(ses.best[1], ses.best[2], ses.best[3])


def polish(probes, theta, iters=6):
    """A few Levenberg-Marquardt steps on the robust cost, from theta. Uses
    only L-only and C-only probes, where the model is a few multiplications
    (what the PIC can afford): returns (theta, cost)"""
    pr = [p for p in probes if p[0] == 0 or p[1] == 0]
    mu = 1e-3
    th = list(theta)
    e = model.cost(th, pr)
    steps = [0.01, 0.01, 0.003]  # relative R, X (ohm), log f

    def resid(q):
        r, x, lf = q
        z, f = complex(r, x), math.exp(lf)
        out = []
        for l, c, sw, rho, floor in pr:
            g = abs(model.gamma_ideal(z, f, l, c, sw))
            out.append(max(0.0, g - floor) if rho <= 0 else g - rho)
        return out

    for _ in range(iters):
        r0 = resid(th)
        jac = []
        hs = [max(abs(th[0]) * steps[0], 0.05), max(abs(th[1]) * steps[1], 0.2), steps[2]]
        for k in range(3):
            q = list(th)
            q[k] += hs[k]
            r1 = resid(q)
            jac.append([(a - b) / hs[k] for a, b in zip(r1, r0)])
        a = [[sum(jac[i][n] * jac[j][n] for n in range(len(r0))) for j in range(3)] for i in range(3)]
        g = [sum(jac[i][n] * r0[n] for n in range(len(r0))) for i in range(3)]
        improved = False
        for _ in range(4):
            m = [[a[i][j] + (mu * a[i][i] if i == j else 0) for j in range(3)] for i in range(3)]
            d = _solve3(m, [-v for v in g])
            if d is None:
                break
            q = [th[i] + d[i] for i in range(3)]
            eq = model.cost(q, pr)
            if eq < e:
                th, e, mu, improved = q, eq, mu / 3, True
                break
            mu *= 10
        if not improved:
            break
    return th, e


def _solve3(m, b):
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
            a[r][c] = b[r]
        out.append(det(a) / d)
    return out


import math


def solve4(t, fallback=True, other_side=True, iters=6):
    """solve3 with the estimate polished by a few robust fitting steps"""
    ses = Session(t)
    for p in PROBES:
        ses.measure(*p, again=False)
    est = model.estimate(ses.probes)
    if est is None or min(p[3] for p in ses.probes) > 0.9:
        for v in SCAN:
            ses.measure(v, 0, 1, again=False)
            ses.measure(0, v, 1, again=False)
        est = model.estimate(ses.probes) or est
    cands = []
    if est:
        th, _ = polish(ses.probes, est[0], iters)
        cands = model.match(th)
    for m in cands:
        ses.measure(*m)
    refine(ses)
    if other_side:
        bsw = ses.best[3]
        for m in cands:
            if m[2] != bsw and ses.seen.get(m, 999) < ses.best[0] * 1.3:
                refine(ses, start=(ses.seen[m],) + m)
    if fallback and ses.best[0] > 300:
        log_grid(t)
        return
    t.set(ses.best[1], ses.best[2], ses.best[3])


def _code(target, of):
    return min(range(128), key=lambda b: abs(of[b] - target))


def solve5(t, fallback=True, factors=(0.7, 1.0, 1.4), polish_iters=0):
    """The closed-form estimate as a map: a small log grid around each
    calculated match (both capacitor sides), then the pattern search"""
    ses = Session(t)
    for p in PROBES:
        ses.measure(*p, again=False)
    est = model.estimate(ses.probes)
    if est is None or min(p[3] for p in ses.probes) > 0.9:
        for v in SCAN:
            ses.measure(v, 0, 1, again=False)
            ses.measure(0, v, 1, again=False)
        est = model.estimate(ses.probes) or est
    if est is None:
        log_grid(t)
        return
    th = est[0]
    if polish_iters:
        th, _ = polish(ses.probes, th, polish_iters)
    starts = []
    for l, c, sw in model.match(th):
        lv, cv = model.L_OF[l], model.C_OF[c]
        local = (10000, l, c, sw)
        for fl in factors:
            for fc in factors:
                m = (_code(lv * fl, model.L_OF), _code(cv * fc, model.C_OF), sw)
                v = ses.measure(*m)
                if v < local[0]:
                    local = (v,) + m
        starts.append(local)
    for st in sorted(starts)[:2]:
        if st[0] < ses.best[0] * 1.3:
            refine(ses, start=st)
    if fallback and ses.best[0] > 300:
        log_grid(t)
        return
    t.set(ses.best[1], ses.best[2], ses.best[3])


def solve6(t, fallback=True, factors=(0.7, 1.0, 1.4), keep=2):
    """Weighted parabola estimates; every root's match measured (the tuner
    decides the capacitor side, not the maths); a small log grid around the
    best ones; then the pattern search"""
    ses = Session(t)
    for p in PROBES:
        ses.measure(*p, again=False)
    if min(p[3] for p in ses.probes) > 0.9:  # nothing moved the SWR: look further
        for v in SCAN:
            ses.measure(v, 0, 1, again=False)
            ses.measure(0, v, 1, again=False)
    ests = model.estimate2(ses.probes, all_candidates=True)
    if not ests:
        log_grid(t)
        return
    matches = []
    for th, _ in ests:
        for m in model.match(th):
            if m not in matches:
                matches.append(m)
    scored = sorted((ses.measure(*m), m) for m in matches)
    starts = []
    for v, (l, c, sw) in scored[:keep]:
        lv, cv = model.L_OF[l], model.C_OF[c]
        local = (v, l, c, sw)
        for fl in factors:
            for fc in factors:
                m = (_code(lv * fl, model.L_OF), _code(cv * fc, model.C_OF), sw)
                w = ses.measure(*m)
                if w < local[0]:
                    local = (w,) + m
        starts.append(local)
    for st in sorted(starts):
        if st[0] < ses.best[0] * 1.3:
            refine(ses, start=st)
    if fallback and ses.best[0] > 300:
        log_grid(t)
        return
    t.set(ses.best[1], ses.best[2], ses.best[3])


def dl8ug(t, grid=(0, 3, 10, 30, 90), keep=3):
    """DL8UG's ATU-10 NG search as his TUNING.md describes it: a coarse grid
    on both capacitor sides, the 3 best points that are not neighbours,
    a pattern search from each (steps a quarter of the value, halved down
    to single relay steps), the best kept"""
    ses = Session(t)
    pts = []
    for sw in (0, 1):
        for l in grid:
            for c in grid:
                pts.append((ses.measure(l, c, sw, again=False), l, c, sw))
    pts.sort()
    chosen = []
    gi = {v: i for i, v in enumerate(grid)}
    for p in pts:
        if all(not (p[3] == q[3] and abs(gi[p[1]] - gi[q[1]]) <= 1 and abs(gi[p[2]] - gi[q[2]]) <= 1) for q in chosen):
            chosen.append(p)
        if len(chosen) == keep:
            break
    for st in chosen:
        big = max(1, max(st[1], st[2]) // 4)
        steps = []
        s = 1
        while s <= big:
            steps.insert(0, s)
            s *= 2
        refine(ses, steps=tuple(steps), start=st)
    t.set(ses.best[1], ses.best[2], ses.best[3])


def _lsq(rows, ys):
    """Least squares for a small system (normal equations, Gauss)"""
    n = len(rows[0])
    a = [[sum(r[i] * r[j] for r in rows) for j in range(n)] for i in range(n)]
    b = [sum(r[i] * y for r, y in zip(rows, ys)) for i in range(n)]
    for i in range(n):  # Gauss with partial pivoting
        p = max(range(i, n), key=lambda k: abs(a[k][i]))
        if abs(a[p][i]) < 1e-12:
            return None
        a[i], a[p], b[i], b[p] = a[p], a[i], b[p], b[i]
        for k in range(i + 1, n):
            f = a[k][i] / a[i][i]
            for j in range(i, n):
                a[k][j] -= f * a[i][j]
            b[k] -= f * b[i]
    x = [0.0] * n
    for i in reversed(range(n)):
        x[i] = (b[i] - sum(a[i][j] * x[j] for j in range(i + 1, n))) / a[i][i]
    return x


def qrefine(ses, start=None, rel=0.25, jumps=6):
    """Newton-like local search: |gamma|^2 near the match is a quadratic
    bowl in (dL, dC). Measure a 3x3 patch (steps rel of the values, at least
    one relay step), fit the bowl, jump to its bottom; again with the patch
    shrunk, until the steps are single relays"""
    local = start or ses.best
    _, l0, c0, sw = local

    def rho2(l, c):
        ses.measure(l, c, sw)
        # the latest probe for (l, c, sw)
        for p in reversed(ses.probes):
            if p[:3] == (l, c, sw):
                r = p[3] if p[3] > 0 else p[4] / 2
                return r * r
        return 1.0

    for _ in range(jumps):
        _, l0, c0, _ = local
        lv, cv = model.L_OF[l0], model.C_OF[c0]
        sl = max(1, round(rel * l0)) if l0 else 2
        sc = max(1, round(rel * c0)) if c0 else 2
        pts = []
        for i in (-1, 0, 1):
            for j in (-1, 0, 1):
                l, c = l0 + i * sl, c0 + j * sc
                if 0 <= l <= 127 and 0 <= c <= 127:
                    v = ses.measure(l, c, sw)
                    if v < local[0]:
                        local = (v, l, c, sw)
                    pts.append((i, j, rho2(l, c)))
        if len(pts) < 6:
            break
        q = _lsq([(1, x, y, x * x, x * y, y * y) for x, y, _ in pts], [g for *_, g in pts])
        if q:
            a0, ax, ay, axx, axy, ayy = q
            det = 4 * axx * ayy - axy * axy
            if axx > 0 and det > 0:  # a bowl: its bottom
                x = (-2 * ayy * ax + axy * ay) / det
                y = (-2 * axx * ay + axy * ax) / det
                x, y = max(-3, min(3, x)), max(-3, min(3, y))
                l, c = round(l0 + x * sl), round(c0 + y * sc)
                l, c = max(0, min(127, l)), max(0, min(127, c))
                v = ses.measure(l, c, sw)
                if v < local[0]:
                    local = (v, l, c, sw)
        if sl == 1 and sc == 1 and local[1:3] == (l0, c0):
            break
        if local[1:3] == (l0, c0):
            rel /= 2  # no better point: look closer
        if local[0] < 105:
            break
    return local


def solve7(t, fallback=True, factors=(0.7, 1.0, 1.4), keep=2):
    """solve6 with the quadratic-bowl local search, then single relay steps"""
    ses = Session(t)
    for p in PROBES:
        ses.measure(*p, again=False)
    if min(p[3] for p in ses.probes) > 0.9:
        for v in SCAN:
            ses.measure(v, 0, 1, again=False)
            ses.measure(0, v, 1, again=False)
    ests = model.estimate2(ses.probes, all_candidates=True)
    if not ests:
        log_grid(t)
        return
    matches = []
    for th, _ in ests:
        for m in model.match(th):
            if m not in matches:
                matches.append(m)
    scored = sorted((ses.measure(*m), m) for m in matches)
    for v, m in scored[:keep]:
        if v < ses.best[0] * 1.5 or v == scored[0][0]:
            qrefine(ses, start=(v,) + m)
    refine(ses, steps=(2, 1))
    if fallback and ses.best[0] > 300:
        log_grid(t)
        return
    t.set(ses.best[1], ses.best[2], ses.best[3])


def solve7_thr(t, thr=300, keep_best=True):
    """solve7 with the log-grid fallback above thr (SWR x100)"""
    ses = Session(t)
    for p in PROBES:
        ses.measure(*p, again=False)
    if min(p[3] for p in ses.probes) > 0.9:
        for v in SCAN:
            ses.measure(v, 0, 1, again=False)
            ses.measure(0, v, 1, again=False)
    ests = model.estimate2(ses.probes, all_candidates=True)
    if not ests:
        log_grid(t)
        return
    matches = []
    for th, _ in ests:
        for m in model.match(th):
            if m not in matches:
                matches.append(m)
    scored = sorted((ses.measure(*m), m) for m in matches)
    for v, m in scored[:2]:
        if v < ses.best[0] * 1.5 or v == scored[0][0]:
            qrefine(ses, start=(v,) + m)
    refine(ses, steps=(2, 1))
    if ses.best[0] > thr:  # the grid as well; keep the better of the two
        log_grid(t)
        if keep_best and t.swr100() > ses.best[0]:
            t.set(ses.best[1], ses.best[2], ses.best[3])
        return
    t.set(ses.best[1], ses.best[2], ses.best[3])


def _vertex(ses, line):
    """Where the fitted parabola along the L line (line 0) or the C line
    (line 1) has its bottom, as a relay code, or None"""
    pts = []
    for p in ses.probes:
        on = p[1] == 0 if line == 0 else p[0] == 0
        if not on or p[2] != 1:
            continue
        code = p[0] if line == 0 else p[1]
        x = (model.L_OF if line == 0 else model.C_OF)[code] * 1e-3
        rho = min(p[3] if p[3] > 0 else p[4] / 2, 0.995)
        sig = model.sigma_rho(rho, p[4]) if p[3] > 0 else p[4] / 2
        s = (1 + rho * rho) / (1 - rho * rho)
        pts.append((x, s, max(4 * rho / (1 - rho * rho) ** 2 * sig, 1e-3)))
    if len(pts) < 3:
        return None
    c = model._parabola(pts)
    if not c or c[2] <= 0:
        return None
    x = -c[1] / (2 * c[2])
    if x <= 0:
        return None
    return _code(x * 1e3, model.L_OF if line == 0 else model.C_OF)


def solve8(t, thr=130, rounds=2, spread=(0.75, 1.0, 1.33), grid='log_grid'):
    """solve7_thr, with probes added where each line's parabola has its
    bottom before the estimate (the readings there tell the most)"""
    ses = Session(t)
    for p in PROBES:
        ses.measure(*p, again=False)
    if min(p[3] for p in ses.probes) > 0.9:
        for v in SCAN:
            ses.measure(v, 0, 1, again=False)
            ses.measure(0, v, 1, again=False)
    for _ in range(rounds):
        added = False
        for line in (0, 1):
            v = _vertex(ses, line)
            if v is None:
                continue
            of = model.L_OF if line == 0 else model.C_OF
            for f in spread:
                code = _code(of[v] * f, of)
                key = (code, 0, 1) if line == 0 else (0, code, 1)
                if key not in ses.seen:
                    ses.measure(*key, again=False)
                    added = True
        if not added:
            break
    ests = model.estimate2(ses.probes, all_candidates=True)
    if not ests:
        log_grid(t)
        return
    matches = []
    for th, _ in ests:
        for m in model.match(th):
            if m not in matches:
                matches.append(m)
    scored = sorted((ses.measure(*m), m) for m in matches)
    for v, m in scored[:2]:
        if v < ses.best[0] * 1.5 or v == scored[0][0]:
            qrefine(ses, start=(v,) + m)
    refine(ses, steps=(2, 1))
    if ses.best[0] > thr:
        globals()[grid](t)
        if t.swr100() > ses.best[0]:
            t.set(ses.best[1], ses.best[2], ses.best[3])
        return
    t.set(ses.best[1], ses.best[2], ses.best[3])
