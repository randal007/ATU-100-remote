"""Where a search fails: by band, and the worst cases"""
import sys
import lab
import searches


def detail(alg, real=True, noise=0.01):
    out = []
    for i, (mhz, r, x, best) in enumerate(lab.loads()):
        t = lab.Tuner(complex(r, x), mhz * 1e6, real=real, noise=noise, seed=1 + i)
        alg(t)
        out.append((mhz, r, x, best, t.true_swr(), t.time, t.n, (t.l, t.c, t.sw)))
    return out


if __name__ == '__main__':
    alg = getattr(searches, sys.argv[1])
    rows = detail(alg)
    bands = sorted(set(r[0] for r in rows))
    for b in bands:
        rs = [r for r in rows if r[0] == b]
        bad = [r for r in rs if r[4] >= 1.5]
        print(f'{b:6.2f} MHz: {len(rs):4d} loads, {len(bad):3d} not below 1.5, mean time {sum(r[5] for r in rs)/len(rs):.2f} s')
    print('worst:')
    for r in sorted(rows, key=lambda r: r[4] - r[3], reverse=True)[:12]:
        print(f'  {r[0]:5.2f} MHz  Z={r[1]:4.0f}{r[2]:+5.0f}j  best {r[3]:.2f}  got {r[4]:6.2f}  at L,C,sw={r[7]}  {r[6]} meas')
