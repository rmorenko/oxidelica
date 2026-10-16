#!/usr/bin/env python3
"""Two solvers agreeing on a model is the witness that it does not lie.

    scripts/solver_pair.py <pair.tsv> <reference-out> <second-list-out> [road]
    scripts/solver_pair.py --merge <reference-a> <reference-b> <reference-out>
    scripts/solver_pair.py --check <reference> <run.tsv>

The first form reads what `scripts/solver_pair.sh` printed for two
solvers and sorts every model into one of four outcomes, each counted
and named on its own:

1. agreed - both ran it to its stop time and agree today: the same
   count of events, to the unit, and the same final point within a
   relative 5 percent with an absolute floor of 1e-6 (the final point
   is printed to six decimals). These go to the reference.
2. apart - both ran it and they disagree. Named with the size of the
   disagreement: the models about which nobody knows yet whether they
   lie, and the queue of work.
3. no check - neither ran into the CPU ceiling, and one side or both
   refused, so there was nothing to compare. Named with the refusal.
4. ceiling - either side outlasted the CPU ceiling. Named, with the
   ceiling that stood.

Three and four are not disagreements, and counting them as such would
swell the second outcome and hide why a model is not covered.

The second form holds one solver's run against the reference by the
same rule and names every model whose fingerprint moved, or that no
longer runs. It is the instrument; the pair is only its admission.

There are two roads into the reference, and they catch different
faults. Two solvers - `dopri45` against `rk4` - agreeing is blind to
an error both share, a wrong equation giving the same wrong answer on
either. One solver against itself at a finer tolerance - `dopri45`
against `dopri45@1e-8` - is blind to whatever does not depend on the
size of the step, and catches what does, which is the class a lost
crossing belongs to and the class the second solver most often fails
to run at all. Neither replaces the other. The pair's sides are named
by the column `solver_pair.sh` writes, so the two sides of the second
road must carry different names - `dopri45` and `dopri45@1e-8` - or
they read as one side.

The optional fourth argument names the road a reference row was
admitted by: `solvers`, `tolerance`. `--merge` puts two references
together, a model admitted by both roads marked `both`, the stronger
guarantee; a row that both hold keeps the first reference's
fingerprint, so a row of the older road is never rewritten by the
newer one. The check runs the side that sorts first, the solver at
the model's own tolerance, which is the side both roads fingerprint.

The reference holds, per model, the count of events, the first
REFERENCE_COLUMNS values of the final point, and the road it came by. The columns are printed
states first, so the first of them are what the run carried; the rest
are algebraics and discretes worked out from them.
"""

import re
import sys

REL = 0.05
ABS = 1e-6
REFERENCE_COLUMNS = 24
PAIR = re.compile(r"(\S.*?) = (-?[0-9.eE+-]+|NaN|nan|-?inf)")


def final_point(text):
    """The `name = value` pairs of a final point. A name may hold a
    comma inside its subscript - `frame.R.T[1, 3]` - so the pairs are
    split on the commas outside brackets, not on every comma."""
    prefix = "final point: "
    if text.startswith(prefix):
        text = text[len(prefix):]
    pairs, depth, start = [], 0, 0
    for at, ch in enumerate(text + ","):
        if ch == "[":
            depth += 1
        elif ch == "]":
            depth -= 1
        elif ch == "," and depth == 0:
            item = text[start:at].strip()
            start = at + 1
            name, sep, value = item.rpartition(" = ")
            if sep and PAIR.fullmatch(item):
                pairs.append((name, value))
    return pairs


def apart(pa, pb):
    """How many values of `pa` the other side holds and disagrees on,
    and the worst of them as (relative, name, a, b)."""
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
    return parted, worst


def read_runs(path):
    runs, order = {}, []
    for line in open(path):
        parts = line.rstrip("\n").split("\t")
        if len(parts) != 6:
            sys.exit(f"solver_pair.py: a line without six columns: {line[:120]!r}")
        cls, solver, status, seconds, events, rest = parts
        if cls not in runs:
            runs[cls] = {}
            order.append(cls)
        runs[cls][solver] = (status, int(seconds), events, rest)
    return runs, order


ROADS = ("solvers", "tolerance", "both")
HEADER = "# class\tevents\tfinal point (first columns, states first)\troad\n"


def read_reference(path):
    """The rows of a reference as (class, events, point, road). A row
    without four columns is not read around: a check that skipped it
    would hold the reference on fewer models than it names."""
    rows = []
    for line in open(path):
        if line.startswith("#") or not line.strip():
            continue
        parts = line.rstrip("\n").split("\t")
        if len(parts) != 4 or parts[3] not in ROADS:
            sys.exit(f"solver_pair.py: a reference row without a named road: {line[:120]!r}")
        rows.append(tuple(parts))
    return rows


def merge(a_path, b_path, out_path):
    a, b = read_reference(a_path), read_reference(b_path)
    b_by = {row[0]: row for row in b}
    rows, both = [], 0
    for cls, events, point, road in a:
        if cls in b_by and b_by[cls][3] != road:
            road = "both"
            both += 1
        rows.append((cls, events, point, road))
    held = {row[0] for row in a}
    rows += [row for row in b if row[0] not in held]
    with open(out_path, "w") as out:
        out.write(HEADER)
        for row in sorted(rows):
            out.write("\t".join(row) + "\n")
    print(f"reference {len(rows)}: {len(a)} from {a_path}, "
          f"{len(rows) - len(a)} added from {b_path}, {both} by both roads")


def admit(pair_path, reference_path, second_path, road="solvers"):
    if road not in ROADS[:2]:
        sys.exit(f"solver_pair.py: road `{road}` is neither of {ROADS[:2]}")
    runs, order = read_runs(pair_path)
    agreed, second, no_check, ceiling = [], [], [], []
    for cls in sorted(order):
        sides = runs[cls]
        if len(sides) != 2:
            sys.exit(f"solver_pair.py: {cls} has {len(sides)} solver line(s), not 2")
        (sa, a), (sb, b) = sorted(sides.items())
        if a[0] == "timeout" or b[0] == "timeout":
            ceiling.append((cls, f"{sa} {a[0]}, {sb} {b[0]}",
                            a[3] if a[0] == "timeout" else b[3]))
            continue
        if a[0] != "ran" or b[0] != "ran":
            no_check.append((cls, f"{sa} {a[0]}, {sb} {b[0]}",
                             a[3] if a[0] != "ran" else b[3]))
            continue
        pa, pb = final_point(a[3]), final_point(b[3])
        ea, eb = int(a[2]), int(b[2])
        parted, worst = apart(pa, pb)
        if ea != eb or parted:
            second.append((cls, ea, eb, parted, worst))
        else:
            point = [(n, v) for n, v in pa if n != "time"][:REFERENCE_COLUMNS]
            agreed.append((cls, ea, point))

    with open(reference_path, "w") as out:
        out.write(HEADER)
        for cls, events, point in agreed:
            out.write(f"{cls}\t{events}\t" + ", ".join(f"{n} = {v}" for n, v in point)
                      + f"\t{road}\n")
    with open(second_path, "w") as out:
        out.write("# class\tevents a\tevents b\tvalues apart\tworst: relative, name, a, b\n")
        for cls, ea, eb, parted, (rel, name, xa, xb) in sorted(
                second, key=lambda s: (s[1] == s[2], -s[4][0])):
            out.write(f"{cls}\t{ea}\t{eb}\t{parted}\t{rel:.3g} {name} {xa} {xb}\n")
        for cls, why, detail in no_check:
            out.write(f"{cls}\tno check\t{why}\t{detail[:200]}\n")
        for cls, why, detail in ceiling:
            out.write(f"{cls}\tceiling\t{why}\t{detail[:200]}\n")

    events_apart = sum(1 for s in second if s[1] != s[2])
    print(f"models {len(order)}: agreed {len(agreed)}, apart {len(second)} "
          f"(events apart {events_apart}, final point only {len(second) - events_apart}), "
          f"no check {len(no_check)}, ceiling {len(ceiling)}")


def check(reference_path, run_path):
    runs, _ = read_runs(run_path)
    held, moved = 0, []
    for cls, events, point, _road in read_reference(reference_path):
        sides = runs.get(cls)
        if not sides:
            moved.append(f"{cls}: not in the run")
            continue
        status, _, got_events, rest = sides[sorted(sides)[0]]
        if status != "ran":
            moved.append(f"{cls}: {status} - {rest[:160]}")
            continue
        if int(got_events) != int(events):
            moved.append(f"{cls}: {got_events} events where the reference holds {events}")
            continue
        parted, (rel, name, xa, xb) = apart(final_point(point), final_point(rest))
        if parted:
            moved.append(f"{cls}: {parted} value(s) moved, worst {name} {xa} -> {xb} ({rel:.3g})")
            continue
        held += 1
    for item in moved:
        print(item)
    print(f"reference held on {held}, moved on {len(moved)}")
    return 1 if moved else 0


def main():
    if len(sys.argv) == 4 and sys.argv[1] == "--check":
        sys.exit(check(sys.argv[2], sys.argv[3]))
    if len(sys.argv) == 5 and sys.argv[1] == "--merge":
        merge(*sys.argv[2:])
        return
    if len(sys.argv) in (4, 5):
        admit(*sys.argv[1:])
        return
    sys.exit(__doc__)


if __name__ == "__main__":
    main()
