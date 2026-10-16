#!/usr/bin/env python3
"""Read what `scripts/solver_pair.sh` printed and say where two solvers agree.

    scripts/solver_pair.py <pair.tsv> <reference-out> <second-list-out>

A model enters the reference only if both solvers ran it to its stop
time and agree on it today: the same count of events, to the unit, and
the same final point within a relative 5 percent with an absolute
floor of 1e-6 (the final point is printed to six decimals). A model the
two disagree on goes to the second list with the size of the
disagreement: those are the models about which nobody knows yet
whether they lie. A model either solver did not finish - the CPU
ceiling or a refusal - is named on its own, never counted as agreeing.

The reference holds, per model, the count of events and the first
REFERENCE_COLUMNS values of the final point. The columns are printed
states first, so the first of them are what the run carried; the rest
are algebraics and discretes worked out from them.
"""

import re
import sys

REL = 0.05
ABS = 1e-6
REFERENCE_COLUMNS = 24
PAIR = re.compile(r"([^,=\s][^,=]*?) = (-?[0-9.eE+-]+|NaN|-?inf)")


def final_point(text):
    if not text.startswith("final point: "):
        return None
    return PAIR.findall(text[len("final point: "):])


def main():
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    runs = {}
    order = []
    for line in open(sys.argv[1]):
        parts = line.rstrip("\n").split("\t")
        if len(parts) != 6:
            sys.exit(f"solver_pair.py: a line without six columns: {line[:120]!r}")
        cls, solver, status, seconds, events, rest = parts
        if cls not in runs:
            runs[cls] = {}
            order.append(cls)
        runs[cls][solver] = (status, int(seconds), events, rest)

    agreed, second, unfinished = [], [], []
    for cls in sorted(order):
        sides = runs[cls]
        if len(sides) != 2:
            sys.exit(f"solver_pair.py: {cls} has {len(sides)} solver line(s), not 2")
        (sa, a), (sb, b) = sorted(sides.items())
        if a[0] != "ran" or b[0] != "ran":
            unfinished.append((cls, f"{sa} {a[0]}, {sb} {b[0]}",
                               a[3] if a[0] != "ran" else b[3]))
            continue
        pa, pb = final_point(a[3]), final_point(b[3])
        if pa is None or pb is None:
            unfinished.append((cls, "no final point", ""))
            continue
        ea, eb = int(a[2]), int(b[2])
        db = dict(pb)
        worst = (0.0, "", 0.0, 0.0)
        parted = 0
        for name, x in pa:
            if name == "time" or name not in db:
                continue
            try:
                xa, xb = float(x), float(db[name])
            except ValueError:
                continue
            if xa != xa or xb != xb:
                if (xa != xa) != (xb != xb):
                    parted += 1
                    worst = max(worst, (float("inf"), name, xa, xb))
                continue
            gap = abs(xa - xb)
            rel = gap / max(abs(xa), abs(xb), ABS)
            if rel > REL and gap > ABS:
                parted += 1
                worst = max(worst, (rel, name, xa, xb))
        if ea != eb or parted:
            second.append((cls, ea, eb, parted, worst))
        else:
            agreed.append((cls, ea, [(n, v) for n, v in pa if n != "time"][:REFERENCE_COLUMNS]))

    with open(sys.argv[2], "w") as out:
        out.write("# class\tevents\tfinal point (first columns, states first)\n")
        for cls, events, point in agreed:
            out.write(f"{cls}\t{events}\t" + ", ".join(f"{n} = {v}" for n, v in point) + "\n")
    with open(sys.argv[3], "w") as out:
        out.write("# class\tevents a\tevents b\tvalues apart\tworst: relative, name, a, b\n")
        for cls, ea, eb, parted, (rel, name, xa, xb) in sorted(
                second, key=lambda s: (s[1] == s[2], -s[4][0])):
            out.write(f"{cls}\t{ea}\t{eb}\t{parted}\t{rel:.3g} {name} {xa} {xb}\n")
        for cls, why, detail in unfinished:
            out.write(f"{cls}\tunfinished\t{why}\t{detail[:200]}\n")

    events_apart = sum(1 for s in second if s[1] != s[2])
    print(f"models {len(order)}: agreed {len(agreed)}, apart {len(second)} "
          f"(events apart {events_apart}, final point only {len(second) - events_apart}), "
          f"unfinished {len(unfinished)}")


if __name__ == "__main__":
    main()
