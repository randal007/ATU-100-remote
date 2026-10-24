import math, sys, lab, searches, model

def true_best(z, f):
    best = (99, 0, 0, 1)
    for sw in (0, 1):
        for l in range(128):
            for c in range(128):
                s = lab.swr(lab.gamma(z, f, l, c, sw, True))
                if s < best[0]:
                    best = (s, l, c, sw)
    return best

def ratio(a, b):
    return max(a, 1) / max(b, 1)

L = lab.loads()[::12]
stats = {'none': 0, 'side': 0, 'far': 0, 'ok': 0}
for i, (mhz, r, x, b) in enumerate(L):
    z, f = complex(r, x), mhz * 1e6
    tb = true_best(z, f)
    t = lab.Tuner(z, f, seed=1 + i)
    ses = searches.Session(t)
    for p in searches.PROBES:
        ses.measure(*p, again=False)
    est = getattr(model, sys.argv[1] if len(sys.argv) > 1 else 'estimate')(ses.probes)
    if not est:
        stats['none'] += 1; print(f'{mhz:5.2f} {r:4.0f}{x:+5.0f}j NONE  best {tb}'); continue
    ms = model.match(est[0])
    same = [m for m in ms if m[2] == tb[3]]
    if not same:
        stats['side'] += 1; print(f'{mhz:5.2f} {r:4.0f}{x:+5.0f}j SIDE  est R{est[0][0]:.0f} X{est[0][1]:+.0f} f{math.exp(est[0][2])/1e6:.2f} -> {ms}  best {tb}'); continue
    m = same[0]
    el = ratio(model.L_OF[m[0]], model.L_OF[tb[1]]); ec = ratio(model.C_OF[m[1]], model.C_OF[tb[2]])
    far = max(el, 1 / el, ec, 1 / ec) > 1.5
    stats['far' if far else 'ok'] += 1
    if far:
        print(f'{mhz:5.2f} {r:4.0f}{x:+5.0f}j FAR   est R{est[0][0]:.0f} X{est[0][1]:+.0f} f{math.exp(est[0][2])/1e6:.2f} -> {m}  best {tb[1:]} swr {tb[0]:.2f}')
print(stats)
