#!/usr/bin/env bash
# Compares two prints of the library pass over the same set of models.
#
#   scripts/ratio_delta.sh <previous main.txt> <current main.txt>
#
# The ratio band in library_floor.sh holds one print against a
# constant; this holds a print against the print before it. Only the
# models both prints ran are summed, so a model won or lost changes the
# set and not the sums - those are named apart. The sum is the quiet
# number (five green runner prints spread about 8.3%); one model is
# loud (its p99 is about 2x), so per model only a doubling is named,
# and only above OUTLIER_MS in the previous print, where a doubling is
# not a few milliseconds of weather. It prints and never fails on a
# delta: it is an observer, not a judge.
set -euo pipefail

if [[ $# -ne 2 || ! -s "$1" || ! -s "$2" ]]; then
  echo "usage: $0 <previous main.txt> <current main.txt> (both non-empty)" >&2
  exit 2
fi
prev=$1
cur=$2
factor=${OUTLIER_FACTOR:-2}
floor_ms=${OUTLIER_MS:-1000}
top=${OUTLIER_TOP:-15}

awk -v factor="$factor" -v floor_ms="$floor_ms" -v top="$top" '
  function times(line, out,   n, f, i) {
    n = split(line, f, " ")
    for (i = 2; i <= n; i++) {
      if (f[i] == "ms" && f[i + 1] == "flattening") out["f"] = f[i - 1] + 0
      if (f[i] == "ms" && f[i + 1] == "running") out["r"] = f[i - 1] + 0
    }
  }
  FNR == 1 { file++ }
  $1 == "ran" { ran[file, $2] = 1; names[$2] = 1 }
  $1 == "work" {
    delete t; times($0, t)
    fl[file, $2] = t["f"]; ru[file, $2] = t["r"]
  }
  END {
    if (file != 2) { print "ratio_delta: expected two files" > "/dev/stderr"; exit 2 }
    both = 0; only1 = ""; only2 = ""
    for (m in names) {
      a = ((1, m) in ran); b = ((2, m) in ran)
      if (a && b) {
        both++
        if (!((1, m) in fl) || !((2, m) in fl)) { missing++; continue }
        f1 += fl[1, m]; f2 += fl[2, m]; r1 += ru[1, m]; r2 += ru[2, m]
        for (k = 1; k <= 2; k++) {
          x = (k == 1) ? fl[1, m] : ru[1, m]
          y = (k == 1) ? fl[2, m] : ru[2, m]
          if (x >= floor_ms && y >= factor * x) {
            out[++no] = sprintf("%.2f %s %s %.0f -> %.0f ms", y / x, \
              (k == 1 ? "flatten" : "run    "), m, x, y)
          }
        }
      } else if (a) only1 = only1 "  previous only: " m "\n"
      else only2 = only2 "  current only:  " m "\n"
    }
    if (both == 0) { print "ratio_delta: no model ran in both" > "/dev/stderr"; exit 2 }
    printf "delta: %d models ran in both prints", both
    if (missing) printf " (%d without a work line, not summed)", missing
    printf "\n"
    printf "delta: flatten %.0fs -> %.0fs (%.3f); run %.0fs -> %.0fs (%.3f)\n", \
      f1 / 1000, f2 / 1000, (f1 ? f2 / f1 : 0), r1 / 1000, r2 / 1000, (r1 ? r2 / r1 : 0)
    printf "%s%s", only1, only2
    printf "delta: %d per-model doublings (>= %sx, previous >= %s ms)\n", no, factor, floor_ms
    # Sort the outliers by growth, dearest first, without asort.
    for (i = 1; i <= no; i++) for (j = i + 1; j <= no; j++) {
      split(out[i], p, " "); split(out[j], q, " ")
      if (q[1] + 0 > p[1] + 0) { s = out[i]; out[i] = out[j]; out[j] = s }
    }
    for (i = 1; i <= no && i <= top; i++) print "  " out[i]
  }
' "$prev" "$cur"
