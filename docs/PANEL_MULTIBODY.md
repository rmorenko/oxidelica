# Panel findings on the multibody line (partial, interrupted by a restart)

Run id wf_cd6b907b-ee3. Three of four lenses reported; the fourth (how many
models stand behind wall 4) did not finish. Resume with resumeFromRunId.

## Lens 1

The probing agent's arithmetic is wrong. Measured with a Hall-violation witness printed from the matcher itself, the subsystem it fails on at `world.frame_b.R.w[2] = rev.frame_a.R.w[2]` is 23 equations touching 22 unknowns (deficiency exactly one, spread over 22 unknowns), not "one unknown, two equations" — and `rev.frame_a.R.w[2]` is indeed determined elsewhere, by a third equation the pair-of-two reading never saw. That third equation is the Revolute joint solving its orientation chain backwards, because `Connections.rooted(frame_a.R)` is answered as `Connections.isRoot` (connections.rs:609) and so is false for every joint in the library. The verdict is (c): not high index, not a matching bug, and not the equalityConstraint half of the overconstrained mechanism — a wrong answer to `rooted` that makes the flattener emit the wrong equations; forcing the correct answer clears Pendulum's structural singularity outright, though a second wall stands behind it for two-joint chains.

- **A 9-line model (world + Revolute + Body, two connects) reproduces the exact refusal of Modelica.Mechanics.MultiBody.Examples.Elementary.Pendulum, in ~10s rather than ~23s.**
  - evidence: /tmp/mbsmall.mo; `oxidelica simulate /tmp/mbsmall.mo` -> "structurally singular model: cannot differentiate through algebraic variable `rev.frame_a.R.w[3]`, differentiating the equation Ref(\"world.frame_b.R.w[3]\") = Ref(\"rev.frame_a.R.w[3]\")", identical to `library check --only ...Pendulum`
  - confidence: verified
- **`world.frame_b.R.w[2]` is named by exactly two flat equations, but `rev.frame_a.R.w[2]` is named by three — the pair the probe quoted plus `rev.frame_a.R.w[2] = R_rel.T[2,1]*rev.frame_b.R.w[1] + R_rel.T[2,2]*rev.frame_b.R.w[2] + R_rel.T[2,3]*rev.frame_b.R.w[3] + R_rel.w[2]`, written in `rev`.**
  - evidence: `oxidelica why Modelica.Mechanics.MultiBody.Examples.Elementary.Pendulum 'rev.frame_a.R.w[2]'` lists three equations; the same for `world.frame_b.R.w[2]` lists two
  - confidence: verified
- **The subsystem the matcher actually fails on at the quoted equation is 23 equations touching 22 unknowns, not 2 and 1. The whole system at that point is 925 algebraic equations for 925 unknowns and 7 states.**
  - evidence: instrumented reduce_index (OXI_HALL) on /tmp/mbsmall.mo: "round 5: 925 algebraic equations, 925 algebraic unknowns, 7 states / unmatched equation #907: world.frame_b.R.w[2] = rev.frame_a.R.w[2] / witness: 23 equations touch 22 unknowns (0 leaks)"
  - confidence: verified
- **The witness is a sound Hall certificate: every equation in it touches only unknowns inside the alternating-reachable set, checked explicitly.**
  - evidence: probe prints "(0 leaks)" for all seven rounds on /tmp/mbsmall.mo; the check iterates eq_vars of each witness equation against the visited set
  - confidence: verified
- **The matcher is not buggy: try_match at crates/oxidelica-sim/src/compile.rs:461 is a textbook Kuhn augmenting-path maximum matching, and the printed witness independently proves no matching exists over the equations as generated.**
  - evidence: crates/oxidelica-sim/src/compile.rs:461-479 (visited-marked augmenting path), loop at 539-545; witness 23>22 with 0 leaks
  - confidence: verified
- **The refusal text the compiler produces today comes from compile.rs:648-651 (differentiation failed). The text the probe quoted as its fourth wall — "equation ... constrains no state, so index reduction cannot help" — is compile.rs:802-804 in choose_the_victim, reached only after the probe's local differentiation hacks.**
  - evidence: crates/oxidelica-sim/src/compile.rs:648-651 and 802-804; the unhacked binary stops at the first of the two
  - confidence: verified
- **`Connections.rooted` is answered as `Connections.isRoot`: both names read the same map, in which only the chosen root node of each component is true.**
  - evidence: crates/oxidelica-parser/src/flatten/connections.rs:604-611 — `(name == "Connections.isRoot" || name == "Connections.rooted") ... Expr::Bool(roots.get(node).copied().unwrap_or(false))`; roots built at connections.rs:537-590 marking only the chosen root true
  - confidence: verified
- **Measured, `Connections.rooted(rev.frame_a.R)` answers false, so MSL's Revolute takes its `else` branch and writes `frame_a.R = absoluteRotation(frame_b.R, R_rel)` — the chain backwards.**
  - evidence: probe print: "[root] Connections.rooted(rev.frame_a.R) -> false"; Modelica/Mechanics/MultiBody/Joints/Revolute.mo:102 `if Connections.rooted(frame_a.R) then` ... :109 `frame_a.R = Frames.absoluteRotation(frame_b.R, R_rel);`
  - confidence: verified
- **Because the only declared root in a MultiBody model is `world.frame_b.R`, `rooted(frame_a.R)` is false for every rotational joint in the library, so all five MSL call sites take the wrong branch every time.**
  - evidence: Modelica/Mechanics/MultiBody/package.mo:336 `Connections.root(frame_b.R);`; grep finds Connections.rooted only in Joints/Revolute.mo:102, Joints/FreeMotion.mo:147, Joints/Spherical.mo:133, Joints/Internal/InitAngle.mo:30, Parts/FixedRotation.mo:154
  - confidence: verified
- **Counterfactual, same binary under an env switch: answering `Connections.rooted` true (the correct answer for this topology) removes the structural singularity from Pendulum entirely — it then stops at `unknown function to_unit1`, an ordinary missing-function wall.**
  - evidence: OXI_ROOTED=1 on /tmp/mbsmall.mo -> "error: unknown function `to_unit1`"; OXI_ROOTED=1 `library check --only ...Pendulum` -> "1 unknown function `to_unit1`"; to_unit1 is Modelica/Units.mo:1293
  - confidence: verified
- **Under the corrected answer the joint writes the chain forward and the collision disappears: `rev.frame_b.R.w[i] = R_rel.T[i,:]*rev.frame_a.R.w + R_rel.w[i]`, leaving `rev.frame_a.R.w[2]` to the connect equation alone.**
  - evidence: OXI_ROOTED=1 `oxidelica why ...Pendulum 'rev.frame_a.R.w[2]'` now lists the three frame_b equations instead of the one solving frame_a
  - confidence: verified
- **`rooted` is not the whole multibody story: a two-joint chain still refuses with the same differentiation message even with the corrected answer, though index reduction gets 13 rounds deep instead of 7.**
  - evidence: /tmp/mb3.mo under OXI_ROOTED=1 -> "cannot differentiate through algebraic variable `rev1.frame_a.R.w[3]`"; real DoublePendulum unchanged under both settings (`structurally singular model: cannot differentiate through al...`)
  - confidence: verified
- **The verdict is (c): a semantics gap in `Connections.rooted`, not (a) over-determination the overconstrained mechanism should remove upstream, and not (b) a matching bug. This model's connection graph is a tree, so no equalityConstraint residues are owed at all.**
  - evidence: tree topology world.frame_b -> rev.frame_a, rev.frame_b -> body.frame_a in /tmp/mbsmall.mo; connections.rs:609 conflates the two operators; Hall witness with 0 leaks rules out (b)
  - confidence: verified
- **The Modelica specification defines `Connections.rooted(A.R)` as true when A.R is closer to the spanning tree's root than B.R for a `Connections.branch(A.R, B.R)`, which is a different question from `isRoot`.**
  - evidence: specification recalled, not read from a local copy; corroborated internally — under isRoot semantics every MSL joint would take its else branch always, which is what was measured and what breaks the family
  - confidence: inferred

## Lens 2

Park, but park properly — the four repairs are not in one place and two of them are not parked at all, they are in the garbage. Repair (3) is already shipped on main and inert; repairs (1)+(2) survive only as a dropped stash (dangling commit 3b9b4b8, on no branch, prunable by gc); repair (4) has no surviving source anywhere I could find. I reproduced the trade independently from two full corpus runs — 819/363 on HEAD against 819/362 with the series, the run-list diff exactly one line, GenerationOfFMUs, none gained — so shipping means lowering RUN_FLOOR 363→362 and RUNNABLE_RUN_FLOOR 358→357, which has no precedent: in 67 floor-touching commits the only decrease was a correction of a mismeasurement on the day the floors were born, and the nearest analogue (a fix that won one model and cost one) was reverted for exactly that. On GenerationOfFMUs the probing agent's reading is wrong on the facts — the derivative's provenance is not lost, it reaches the gathering in both directions and is discarded by the grounding fixpoint as a cycle — and a 40-line rule that removes that wall uncovers a second one inside the annotation machinery itself, so "ship after repairing it" is not a small precondition.

- **The four repairs are not a single unshipped body of work: repair (3), the aggregate-as-call-argument narrowing, is already committed on main and inert without the others.**
  - evidence: git show 2720510 --stat: crates/oxidelica-parser/src/flatten/names.rs, 36 insertions; message says "Corpus unchanged at 819 and 363 on its own, because nothing keeps a call standing yet"
  - confidence: verified
- **Repairs (1) noDerivative and (2) InlineAfterIndexReduction survive only as a dropped stash — dangling commit 3b9b4b8 ("WIP on main: 7e72e89", 2026-09-05 01:37) — which is on no branch and in no stash list, and which git gc deletes after two weeks by default.**
  - evidence: git fsck --no-reflogs --unreachable finds 3b9b4b8; git reflog stash lists only stash@{0} verifier-stash and stash@{1} clocks WIP; git config gc.pruneExpire and gc.auto are both unset (defaults 2.weeks.ago / 6700 loose objects, currently 3213 per git count-objects -v)
  - confidence: verified
- **Repair (4), restoring the record's name at both binding roads, has no surviving source anywhere in the object store — it exists only as prose in docs/ANALYSIS.md and commit 4da30b3, which touched docs only.**
  - evidence: git show 4da30b3 --stat: docs/ANALYSIS.md, 35 insertions, no code; the newest dangling commit carrying compiler code is 3b9b4b8 at 01:37 while 4da30b3 is at 11:23; every loose object written after 02:00 on 2026-09-05 belongs to the docs-only commits (find .git/objects -newermt)
  - confidence: verified
- **The "four" measured at 819/362 are not the four the brief names: the fourth in that measurement was the der-gathering attempt in compile.rs, not the record-name restoration, which came later and was never measured on the corpus.**
  - evidence: f4738d9 message: "All four parts together - reading noDerivative, honouring InlineAfterIndexReduction, allowing an aggregate where the input was declared one, and gathering a definition through der(...) - come to 819 flatten and 362 run"; 4da30b3 message: "Nothing shipped ... Floors stay at 819 and 363"
  - confidence: verified
- **The trade is exactly as stated and I reproduced it independently: HEAD gives 819 flatten / 363 run (runnable 721/358); the series gives 819 flatten / 362 run (runnable 721/357).**
  - evidence: two full corpus runs, one per binary: /tmp/list-head.txt "classes: 7631; example models: 1043, of which 819 flatten and 363 run"; /tmp/list-series.txt "...819 flatten and 362 run"
  - confidence: verified
- **By the instrument AGENTS.md demands for a definition-adding change — the diff of which models run — the loss is exactly one model and nothing is gained; the flatten lists are identical at 819.**
  - evidence: diff of the sorted 'ran' lists: single line "154d153 < Modelica.Mechanics.Rotational.Examples.GenerationOfFMUs"; grep -c '^ flat ' gives 819 in both files
  - confidence: verified
- **Shipping the series would require lowering two enforced floors, RUN_FLOOR 363→362 and RUNNABLE_RUN_FLOOR 358→357, and CI fails on either.**
  - evidence: scripts/library_floor.sh sets RUN_FLOOR=363, RUNNABLE_RUN_FLOOR=358 and exits non-zero via short() when the count is below; .github/workflows/ci.yml:98 runs ./scripts/library_floor.sh .msl
  - confidence: verified
- **No floor in this repository has ever been lowered to accommodate a change. The single decrease in the whole history (FLATTEN 388→381, RUN 37→36) was a correction of a mismeasurement on the day the floors were introduced.**
  - evidence: 67 commits touch scripts/library_floor.sh; RUN_FLOOR history is 37 36 38 39 41 51 91 ... 358 363; the one decrease is commit 0496c00 "The numbers were measured against the wrong library" (2026-08-22, same day as d1c3885 which introduced them)
  - confidence: verified
- **The closest precedent in the log is a change that won one model and cost one, and it was reverted for exactly that — net zero was not good enough.**
  - evidence: dd9afb6 "Take the clocked if, measure it, and put it back with what it uncovered": "The corpus says 772 and 337 - plus one running, minus one flattening - so it went back" and "Taking the first alone costs a model to win a model"
  - confidence: verified
- **A second precedent: a guard that was correct and printed the right diagnosis by name was taken out of the tree for costing thirty-three models.**
  - evidence: 263a11b "Measure the scalar-input guard and leave it out": "Then the library check: 633 flatten, down from 666 ... Out of the tree, into the register"
  - confidence: verified
- **A third: the law in AGENTS.md was itself written after three correct definition-adding rules were reverted, the last measured 363 off against 354 on from one binary.**
  - evidence: afbe7fc "Name the law behind three reverts": "Measured from one binary the third attempt is smaller than it looked: 363 off, 354 on"; the law text is AGENTS.md lines 27-55
  - confidence: verified
- **I searched all 641 commit bodies for any admission of a model lost in a shipped change and found none — every hit belongs to this multibody line's own commits.**
  - evidence: git log --format='%h|%ad|%s|%b' piped through grep -inE 'lost (one|two|a model)|loses (one|two|exactly)|costs? (one|...) model|in exchange|net (gain|of)' returns 5 lines, all from 610257b, f4738d9, 2720510 and two unrelated (register rows, a speed remark)
  - confidence: verified
- **GenerationOfFMUs runs on HEAD and refuses under the series with a precisely reproducible message naming w_internal.**
  - evidence: HEAD binary --only: "1 flatten and 1 run"; worktree binary with the stash patch applied: "structurally singular model: cannot differentiate through algebraic variable `torqueToAngle2b.w_internal`, differentiating the equation Bin(Sub, Ref("torqueToAngle2b.w_internal"), Ref("inertia2b.w")) = Number(0.0)"
  - confidence: verified
- **The probing agent's reading is wrong on the facts: the derivative does NOT lose its provenance. Both `w_internal = der(phi)` and `w_internal = w` reach the candidate gathering, and the der form reaches it in both orientations.**
  - evidence: instrumented reduce_index in my worktree: "PROBE cand torqueToAngle2b.w_internal := Ref(\"der(torqueToAngle2b.phi)\")", "PROBE cand der(torqueToAngle2b.phi) := Ref(\"torqueToAngle2b.w_internal\")", and the equation is in algebraic_eqs as Ref("der(torqueToAngle2b.phi)") = Ref("torqueToAngle2b.w_internal")
  - confidence: verified
- **What actually discards the provenance is the grounding fixpoint: every road out of w_internal is a cycle (w_internal↔der(phi), w_internal↔w) except the one equation under reduction, which the gathering skips by design.**
  - evidence: crates/oxidelica-sim/src/compile.rs:615-636, a candidate is accepted only when every unknown it names is already accepted; compile.rs:~575 "if index == eq { continue; }"; the surviving sibling torqueToAngle2a gets alg_defs[torqueToAngle2a.w_internal] = Ref("inertia2a.w") because its connection equation is not the one being reduced
  - confidence: verified
- **The size of the provenance repair, measured rather than guessed: one rule of about forty lines in the gathering removes the named wall, and the model still does not run — the wall behind it is an underdetermined algebraic loop.**
  - evidence: added a DER_RULE=1 switch to the fixpoint (an unknown spelled der(x) is the time derivative of whatever determines x), one binary two runs: off gives "cannot differentiate through algebraic variable torqueToAngle2b.w_internal", on gives "underdetermined algebraic loop [\"springDamper.angleToTorque2.move_w.u[2]\"]"
  - confidence: verified
- **The second wall sits inside the annotation machinery repairs (1) and (2) add, not outside it: the named variable belongs to Move, whose position function carries the very pair of annotations the series honours.**
  - evidence: the refusal names springDamper.angleToTorque2.move_w.u[2]; Modelica/Mechanics/Rotational/Sources/Move.mo:22 reads annotation(derivative(noDerivative=q_qd_qdd) = position_der, InlineAfterIndexReduction=true)
  - confidence: inferred
- **So the answer to "field, pass or architecture" is none of the three as posed: the provenance is already in the table, one grounding rule throws it away, and repairing GenerationOfFMUs is a chain of at least two links whose bottom I did not reach.**
  - evidence: the two measurements above, taken with the ladder AGENTS.md prescribes (small first, then library check --only, one second per answer); I stopped at rung two rather than guess at the depth
  - confidence: inferred
- **Resumption from the dropped stash is free today: the patch applies to HEAD without fuzz, builds, and reproduces 819/362 exactly.**
  - evidence: git apply of the 106-line diff 7e72e89..3b9b4b8 onto a worktree at 610257b succeeded silently; cargo build --release succeeded; the corpus run gave 819/362
  - confidence: verified
- **But it will not stay free: all three files the patch touches are hot, so an unparked patch is a patch that stops applying.**
  - evidence: since 2026-08-22: inlining.rs 22 commits, compile.rs 22 commits, classes.rs 6 commits (git log --oneline --since per file)
  - confidence: verified
- **Parking properly is one command plus two lines, not a shift: branch the dangling stash, leave repair (3) where it is, and re-derive repair (4) from the map, whose site is already identified in the code.**
  - evidence: git branch <name> 3b9b4b8 preserves the two annotation repairs; 4da30b3 calls repair (4) "two lines" and names both roads, which are the Array branch that does `position += 1; continue;` and the by-name branch at crates/oxidelica-parser/src/flatten/inlining.rs:1633-1657, where `bindings.insert(input.name.clone(), arg.clone())` is skipped by the continue
  - confidence: verified
- **docs/ANALYSIS.md does not rot — it is committed and carries the whole map: the four walls in order, every site, and the last measurement rather than the last guess.**
  - evidence: docs/ANALYSIS.md, 4067 lines, sections "The series, measured whole", "Where the fourth appearance has to be fixed", "The chain rule applied at last"; AGENTS.md lines 288-294 require exactly this handover
  - confidence: verified
- **The case against my recommendation, stated once: AGENTS.md explicitly permits a definition-adding change to cost models if the victims are measured and the change is argued — and here the victims were measured and the loss is one named model. The compiler is presently ignoring an annotation the library wrote and reading past a noDerivative it should read.**
  - evidence: AGENTS.md lines 50-55: "a definition-adding change is not wrong for costing models, and not right for taking a small one: measure the victims, and if they move, the change is a different compiler rather than a fix"
  - confidence: verified
- **I still land on park because that clause asks for a reason the new selection is better, not merely different — and today it is only different: one model worse, none better, all sixteen multibody models still at a wall, and the line's own last shift concluding the bottom is matching a redundant pair rather than anything these four repairs supply.**
  - evidence: 610257b: "The multibody family does not need a derivative rule for rotations or a record travelling into one. It needs matching that can handle a connection between two things already determined"; the corpus diff shows no model gained
  - confidence: inferred

## Lens 3

Oxidelica implements the first half of MLS 9.4 and not the second. The clauses parse, the graph is built, one root per component is chosen, and the model is instantiated twice so the graph can be asked - but `equalityConstraint` appears nowhere in the repository, no spanning tree is ever oriented, and `Connections.rooted` is answered from the same map as `Connections.isRoot`, which is a different operator by the MSL's own reference documentation. Both gaps are measurable and each has its own family: the `rooted` conflation flips `Revolute` into its inverted branch on every open chain (measured: with a correct answer the Pendulum's wall moves from "structurally singular" to an unrelated missing function), and the missing `equalityConstraint` leaves exactly 9 surplus equations per closed kinematic loop - 12 components of `Frames.Orientation` where 3 residues are owed - measured on Fourbar1 (2002 for 1993) and reproduced in a 14-line model (1070 for 1061, zero surplus with the loop opened). The specific pair the probing agent reported is not itself a symptom of either: `connect(world.frame_b, rev.frame_a)` is a spanning-tree edge, full component equality is exactly what 9.4 requires there, and oxidelica generates exactly that. Its reading "one unknown, two equations" is wrong - the model is balanced; what was wrong beside it was the joint's causality.

- **Oxidelica has the first half of the overconstrained mechanism: `Connections.root/potentialRoot/branch` parse into `GraphClause`, `choose_roots` builds the node/edge set and picks exactly one root per connected component (refusing on two declared roots or none), and `build_the_model` instantiates the model twice so that graph questions can be answered on the second pass.**
  - evidence: /Users/romanmorenko/oxidelica/crates/oxidelica-parser/src/parser/equations.rs:575-619; /Users/romanmorenko/oxidelica/crates/oxidelica-parser/src/flatten/connections.rs:430-593; /Users/romanmorenko/oxidelica/crates/oxidelica-parser/src/flatten/mod.rs:498-508
  - confidence: verified
- **`equalityConstraint` appears nowhere in the oxidelica repository - not in code, docs or scripts.**
  - evidence: `grep -rni "equalityconstraint" /Users/romanmorenko/oxidelica/ --include="*.rs" --include="*.md" --include="*.sh"` returns no lines (exit 0, empty output)
  - confidence: verified
- **`choose_roots` returns only `HashMap<String, bool>` - "is this node the chosen root". It runs union-find over branch and connect edges but never records which edges are spanning-tree edges and which close a cycle, never counts cycles, and never refuses one. There is no place in the compiler where a closing edge could be identified.**
  - evidence: /Users/romanmorenko/oxidelica/crates/oxidelica-parser/src/flatten/connections.rs:516-592 - the union-find at :532-537 discards the tree it builds; the only value returned at :592 is `chosen`
  - confidence: verified
- **Connect equations are generated in `join_the_connections`, which never consults `acc.roots`: every non-flow, non-stream, non-parameter member of a connection set gets a plain equality against the first member. A cycle-closing connect therefore produces full component equality, 12 equations for a `Frames.Orientation`.**
  - evidence: /Users/romanmorenko/oxidelica/crates/oxidelica-parser/src/flatten/mod.rs:522-892; the potential-equality loop is at :881-890 (`for (other, _) in &members[1..] { ... lhs: var(other), rhs: var(members[0].0) }`)
  - confidence: verified
- **`Connections.isRoot` and `Connections.rooted` are answered from the same lookup in the same map, so `rooted(A)` returns "is A the chosen root".**
  - evidence: /Users/romanmorenko/oxidelica/crates/oxidelica-parser/src/flatten/connections.rs:604-612: `if (name == "Connections.isRoot" || name == "Connections.rooted") && args.len() == 1 => match &args[0] { Expr::Ref(node) => Expr::Bool(roots.get(node).copied().unwrap_or(false)), ...`
  - confidence: verified
- **The two operators are different, and the MSL under test says so itself: isRoot asks whether the instance "is selected as a root in the virtual connection graph", while rooted "returns true, if A.R is closer to the root of the spanning tree than B.R", and its argument must be the first argument of a `Connections.branch`.**
  - evidence: /Users/romanmorenko/.local/share/oxidelica/libraries/Modelica/ModelicaReference/package.mo:3319 (isRoot) and :3339 (rooted)
  - confidence: verified
- **A 25-line model shows the wrong answer directly: `Connections.root(ground.p)`, `Connections.branch(a, b)` inside `Body`, `connect(ground.p, arm.a)`. Oxidelica folds `Connections.rooted(arm.a)` to false; by the reference above it must be true, since arm.a is the branch end nearer the root.**
  - evidence: `oxidelica why /tmp/mb-lens/rooted.mo arm.near` prints `equation: arm.near = if false then 1 else 0`; with an env-gated spanning-tree depth computed from the chosen roots in one rebuilt binary the same command prints `if true`, and `arm.far` stays `if false`
  - confidence: verified
- **The conflation makes `Modelica.Mechanics.MultiBody.Joints.Revolute` take its inverted branch. On Pendulum, oxidelica generates `rev.frame_a.R.w[3] = R_rel.T[3,:]*rev.frame_b.R.w + R_rel.w[3]` - `frame_a.R = absoluteRotation(frame_b.R, R_rel)`, the `else` branch - so the joint tries to define the frame the connect to the world has already fixed.**
  - evidence: `oxidelica why Modelica.Mechanics.MultiBody.Examples.Elementary.Pendulum "rev.frame_a.R.w[3]"`; the source branch is /Users/romanmorenko/.local/share/oxidelica/libraries/Modelica/Modelica/Mechanics/MultiBody/Joints/Revolute.mo:102-112
  - confidence: verified
- **With the spec answer for `rooted`, the direction flips to `rev.frame_b.R.w[i] = R_rel.T[i,:]*rev.frame_a.R.w + R_rel.w[i]` and the Pendulum's wall moves: "structurally singular model: cannot differentiate through algebraic variable rev.frame_a.R.w[3]" becomes "unknown function to_unit1". One binary, one env switch, both numbers from it.**
  - evidence: `library check <MSL> --only ...Elementary.Pendulum` with and without OXIDELICA_ROOTED=1 on the same rebuilt binary; `to_unit1` is Modelica/Units.mo:1293, an unrelated vectorized-call lookup
  - confidence: verified
- **The `rooted` repair moves exactly one of the seven multibody models measured. DoublePendulum, DoublePendulumInitTip, PendulumWithSpringDamper, FreeBody, ForceAndTorque and Fourbar1 keep their refusal word for word with the switch on, even though DoublePendulum's joint equations do flip direction as intended.**
  - evidence: one binary, switch off/on: DoublePendulum keeps "cannot differentiate through algebraic variable `revolute1.frame_a.R.T[3,1]`, differentiating Ref(\"world.frame_b.R.T[3,1]\") = Ref(\"revolute1.frame_a.R.T[3,1]\")" both ways, while `why` shows the joint's equations going from `frame_a.R.T = f(frame_b.R.T)` to `frame_b.R.T = f(frame_a.R.T)`
  - confidence: verified
- **`Frames.Orientation` is the canonical overdetermined record: 12 components (`Real T[3,3]`, `SI.AngularVelocity w[3]`) carrying an `encapsulated function equalityConstraint` with `output Real residue[3]`. `Interfaces.Frame` holds `r_0[3]`, that `R`, and `flow f[3]`, `flow t[3]`; it declares no Connections clause itself - the clauses live in the components (`Connections.root(frame_b.R)` in World, `Connections.branch(frame_a.R, frame_b.R)` in PartialElementaryJoint, `Connections.potentialRoot(frame_a.R)` in Body).**
  - evidence: /Users/romanmorenko/.local/share/oxidelica/libraries/Modelica/Modelica/Mechanics/MultiBody/Frames/Orientation.mo; .../Interfaces/Frame.mo; .../MultiBody/package.mo:336; .../Interfaces/PartialElementaryJoint.mo:11; .../Parts/Body.mo:204-206
  - confidence: verified
- **The missing `equalityConstraint` costs exactly 9 equations per closed kinematic loop - 12 components equated where 3 residues are owed. Fourbar1 (one loop) refuses with "unbalanced model: 2002 algebraic equation(s) for 1993 unknown(s)", a surplus of 9; a hand-built 14-line loop (world, two Revolutes, two FixedTranslations, loop closed) gives 1070 for 1061, and the same model with only the closing connect removed reports no unbalance at all.**
  - evidence: `library check <MSL> --only ...Loops.Fourbar1`; `oxidelica simulate /tmp/mb-lens/loop3.mo` vs the one-line-different control - the two files differed only by `connect(t2.frame_b, world.frame_b);`
  - confidence: verified
- **The tests named in the brief do not cover either gap. `an_overconstrained_graph_is_broken_at_a_root` establishes that a declared root takes its part, that an unknown clause name, a fractional priority, no root and two roots are all refused, and that a potential root serves otherwise - its single use of `rooted` asks about the node that IS the chosen root, so it cannot tell `rooted` from `isRoot`. `a_branch_may_say_where_the_graph_is_rooted` is, despite its name, about a `Connections.root` clause inside an `if`: a parameter condition is accepted, a run-time Boolean is refused with "drawn once and for all". `the_graph_is_drawn_before_it_is_asked` establishes the two-pass build using `isRoot` only. None mentions a spanning tree, a cycle, or `equalityConstraint`.**
  - evidence: /Users/romanmorenko/oxidelica/crates/oxidelica-parser/tests/flattening/connections.rs:285-333 (the `rooted` call is at :327), :867-895, :899-939
  - confidence: verified
- **For `connect(world.frame_b, rev.frame_a)` in Pendulum, MLS 9.4 requires full component equality, not `equalityConstraint`: the model is an open chain, so that connect is a spanning-tree edge. Oxidelica generates exactly that - `world.frame_b.R.w[2] = rev.frame_a.R.w[2]` and the eleven siblings. On this pair the overconstrained mechanism is correctly applied and irrelevant.**
  - evidence: `oxidelica why ...Pendulum "world.frame_b.R.w[2]"` shows only the two equations; corroborated by Pendulum passing the balance check (only Fourbar1 and ForceAndTorque report "unbalanced"), i.e. no closing edge is present to be wrongly expanded
  - confidence: verified
- **The probing agent's local reading of that pair - "one unknown, two equations" - is not what the flat model says: two unknowns and two equations, and the model is balanced. What was wrong beside it was the joint's causality, which is the `rooted` defect.**
  - evidence: `why` output lists `world.frame_b.R.w[2] = 0` and `world.frame_b.R.w[2] = rev.frame_a.R.w[2]` - two distinct variables; Pendulum never triggers the unbalance refusal, and its refusal is the later "structurally singular"
  - confidence: verified
- **The wall that stands in front of most multibody models after `rooted` is corrected is a fourth thing, in index reduction and not in chapter 9: the reducer tries to differentiate `world.frame_b.R.T[i,j] = joint.frame_a.R.T[i,j]`, an equation between two quantities the world fixes by declaration, and refuses because the right side is algebraic. That is the same shape in DoublePendulum and DoublePendulumInitTip and it does not move with the graph.**
  - evidence: identical refusal text with the probe on and off for both models; `world.frame_b.R = Frames.nullRotation()` at MultiBody/package.mo:339 is written as an equation, not a binding
  - confidence: inferred

## Lens 4

The multibody line is not in the tree — only repair (3) shipped (commit 2720510), so wall 4 is unreachable by any binary that exists, and "how many are behind wall 4" cannot be measured for the sixteen: the line's own probe reached it on one model out of sixteen, after three walls knocked down by hand and kept nowhere. What can be measured, and is: wall 4's exact message is already reached today by 5 corpus models — two of them MultiBody, with the identical `world.frame_b.R.T[3,1] = fixedRotation.frame_a.R.T[3,1]` shape — so the wall is real, but no model has ever been walked past it, and the number known to run if it falls is 0. The sixteen are all still at the earlier wall, and their unmatched equations say they are not one family: 15 of 16 are a plain `Ref = Ref` connection equality, but roughly half of those sit in kinematic loops (Fourbar2, PlanarFourbar, Engine1a, the rolling-wheel sets, the Constraints examples) where index reduction is genuinely owed and the matcher is not the answer, while the open chains (Pendulum, DoublePendulum, TestMove and kin) are the redundant-alias shape the probe found. RollingWheel is definitely not a redundant pair — its unmatched equation is a nonlinear rolling constraint.

- **The four repairs of the multibody line are not in the working tree: only repair (3) (aggregate as a call argument) is shipped, as commit 2720510 touching names.rs; every other commit of the line since is documentation only.**
  - evidence: git show --stat for 610257b, 4da30b3, 36a0d33, 4b3976a, 2f1aaf5, 78b474d, f4738d9, c4aac3f, 88d3e4e, df77e9b, 5a90a2e — all `docs/ANALYSIS.md` and/or `AGENTS.md` only; 2720510 is `crates/oxidelica-parser/src/flatten/names.rs | 37 +-`. `git status --porcelain` is empty; grep for InlineAfterIndexReduction/noDerivative in crates/**/*.rs finds only parser comments saying they are read past.
  - confidence: verified
- **The binary at /Users/romanmorenko/oxidelica/target/release/oxidelica is exactly HEAD (610257b) with no experimental switch, so every number below comes from one binary.**
  - evidence: I built 610257b in a private worktree; shasum -a 256 of /tmp/mb-lens/target/release/oxidelica and of the tree's binary are both 12b6f91300d99e9ac0e9343150553771a5aca1e86f1dea2576d2322f41cbb8ef. Corpus totals reproduce the recorded floors: "example models: 1043, of which 819 flatten and 363 run" (/tmp/mb-report.txt line 2).
  - confidence: verified
- **All sixteen models the line was built for currently refuse at the earlier wall — `structurally singular model: cannot differentiate through algebraic variable X` — and not at wall 4.**
  - evidence: /tmp/mb-report.txt, built half: 26 refusals of that message, of which exactly sixteen name a multibody frame quantity: R.T x6 (PrismaticConstraint, DoublePendulum, DoublePendulumInitTip, RollingWheelSetDriving, RollingWheelSetPulling, PlanarFourbar), R.w x6 (Pendulum, InitSpringConstant, Engine1a, Fourbar2, BevelGear1D, ModelicaTest.Rotational.TestMove), r_0 x3 (HeatLosses, PendulumWithSpringDamper, SpringDamperSystem), delta_0 x1 (RollingWheel). Reproduced per model: `library check --only ...BevelGear1D` gives 1 flatten, 0 run, same message.
  - confidence: verified
- **The 6/6/3/1 breakdown reproduces docs/ANALYSIS.md exactly, so my sixteen are the same sixteen the line was built for.**
  - evidence: docs/ANALYSIS.md line ~3527: "the orientation of a multibody frame accounts for twelve: six on `R.T` and six on `R.w`, with three more on `r_0` and one on `delta_0`" — 6+6+3+1 = 16, matching the extraction above name for name.
  - confidence: verified
- **Wall 4's exact message is reached today, without the line, by 5 models corpus-wide.**
  - evidence: /tmp/mb-report.txt: 5 refusals of `structurally singular model: equation ... constrains no state, so index reduction cannot help` — Electrical.QuasiStatic.SinglePhase.Examples.ParallelResonance (`inductor.pin_p.reference.gamma` = `capacitor.pin_p.reference.gamma`), MultiBody.Examples.Constraints.RevoluteConstraint and UniversalConstraint (both `world.frame_b.R.T[3,1]` = `fixedRotation.frame_a.R.T[3,1]`), Rotational and Translational Examples.Utilities.SpringDamperNoRelativeStates (`flange_b.tau` = 0 / `flange_b.f` = 0).
  - confidence: verified
- **Two of those five are MultiBody and carry the identical redundant-pair shape the probe described, reached with no hand-knocked walls: the world's frame is pinned to a constant and a connection passes it on.**
  - evidence: `oxidelica why ...RevoluteConstraint 'world.frame_b.R.T[3,1]'` prints exactly two equations: `world.frame_b.R.T[3,1] = 0` (written in world) and `world.frame_b.R.T[3,1] = fixedRotation.frame_a.R.T[3,1]`, then the constrains-no-state refusal. The same probe on the other side shows five equations naming `fixedRotation.frame_a.R.T[3,1]`, i.e. a whole connection set of frames.
  - confidence: verified
- **Wall 0 and wall 4 are two exits from the same failure — an equation the matching could not use — so the unmatched equation the sixteen die on is already printed today and can be read without rebuilding the line.**
  - evidence: crates/oxidelica-sim/src/compile.rs: reduce_index matches, and on failure first differentiates the failed equation (err at line ~649, "cannot differentiate through algebraic variable") and only then looks for a state to demote (err at line ~803, "constrains no state"). Both messages quote the same failed equation.
  - confidence: verified
- **Fifteen of the sixteen have an unmatched equation of plain `Ref(A) = Ref(B)` connection-equality shape; the sixteenth, RollingWheel, does not — its unmatched equation is a nonlinear rolling constraint, so it is a genuine constraint and not a redundant pair.**
  - evidence: /tmp/mb-report.txt, equations extracted per model: e.g. Pendulum `Ref("world.frame_b.R.w[3]") = Ref("rev.frame_a.R.w[3]")`, HeatLosses `Ref("damper1.frame_b.r_0[3]") = Ref("body1.frame_a.r_0[3]")`, BevelGear1D `Ref("revolute3.frame_a.R.w[1]") = Ref("revolute2.frame_b.R.w[1]")`; RollingWheel `Number(0.0) = Bin(Sub, Ref("wheel1.rollingWheel.radius"), ...delta_0[1]...)`.
  - confidence: verified
- **The sixteen are not one family: roughly nine are open kinematic chains, where a failure to match is a redundant alias and the matcher is the right subject; roughly seven sit in closed loops or true constraints, where index reduction is genuinely owed and a matcher fix will not help them.**
  - evidence: Components used, from the model files: Pendulum and DoublePendulum are Joints.Revolute + Parts.Body only (tree); SpringDamperSystem/PendulumWithSpringDamper/InitSpringConstant add only Forces.Spring/Damper, which do not constrain kinematically; against that, Engine1a is Prismatic+Revolute (crank-slider loop), Fourbar2 uses Joints.UniversalSpherical (a cut joint), PlanarFourbar uses Joints.RevolutePlanarLoopConstraint, RollingWheel/RollingWheelSet* use rolling constraints, and PrismaticConstraint is a Constraints example. BevelGear1D is ambiguous (a BevelGear coupling two revolutes).
  - confidence: inferred
- **NUMBER 1 — behind wall 4 only: 0 models are known to run if wall 4 falls, and 5 models stand at it today (2 of them MultiBody). How many of the sixteen would arrive there is unmeasured: between 1 and 16.**
  - evidence: Method: count of the `constrains no state` message in /tmp/mb-report.txt = 5. Nobody has walked past wall 4 — docs/ANALYSIS.md's last section says the fourth wall "does not fall" and the probe stopped there, on one model, with walls 1-3 "knocked down locally and crudely, nothing kept". Charter (AGENTS.md, "A census entry is not a family"): emptying an entry uncovers whatever stood behind it.
  - confidence: verified
- **NUMBER 2 — behind wall 4 plus the connector-record fault: 6 to 9 models standing, still 0 known to run. The connector-record count itself cannot be established: it is visible only with the reverted line in, and the shift that measured it recorded two example names instead of a count.**
  - evidence: docs/ANALYSIS.md (commit 4da30b3 section): "some still stop on a record's name, but a different one: `fixedFrame.frame_a.R`, `spring1.lineForce.frame_a.R`" — no number given. Grep of the library: `fixedFrame` occurs in exactly two files, one of which is a member of the sixteen (Loops/Fourbar2.mo); `spring1` occurs in three members of the sixteen (HeatLosses, PendulumWithSpringDamper, SpringDamperSystem). So the group is at least 1 and at most 4 of the sixteen by that trace.
  - confidence: uncertain
- **NUMBER 3 — the total multibody family: 16 is the family the line was built for; 56 MultiBody models do not run today; of those 56 only 17 are at the singular kind at all, so 39 stand at walls this line never touched.**
  - evidence: /tmp/mb-report.txt: 56 lines beginning `refused`/`built` whose model name starts `Modelica.Mechanics.MultiBody` (8 refused at flatten, 48 flattened and would not run). Of the 48: 15 `cannot differentiate through algebraic variable` (the sixteenth of the sixteen is ModelicaTest.Rotational.TestMove, outside the name), 2 `constrains no state`, 17 unbalanced, 6 unknown function, 3 `two equations for der(...)`, and a tail of parameters; of the 8 that do not flatten, 4 are a missing `inner world`, 2 are `Connections.isRoot`, 1 a non-constant condition.
  - confidence: verified
- **The kind `still cannot be matched after N index reductions` is empty at HEAD, so the whole matcher-shaped question lives in the 5 `constrains no state` refusals and in whatever the 26 differentiation refusals are hiding.**
  - evidence: grep -c "still cannot be matched" /tmp/mb-report.txt = 0. Total `structurally singular` refusals = 37, split 26 / 6 (`cannot differentiate function`) / 5 (`constrains no state`).
  - confidence: verified
- **An upper bound on the whole redundant-pair-shaped population at HEAD is 23 models; the subset where the shape is likely genuine redundancy rather than genuine high index is about 16 to 17.**
  - evidence: Method: 5 at the `constrains no state` message + 15 of the sixteen whose unmatched equation is a plain connection equality + 3 connected-current models = 23. Subtracting the ~7 loop/constraint members of the sixteen (Engine1a, Fourbar2, PlanarFourbar, RollingWheelSet x2, PrismaticConstraint, and BevelGear1D as doubtful) leaves ~16. The 5 and the 3 are verified by their printed equations; the loop/open split is inferred from model components, so this number is a shape count and not a count of models that would run.
  - confidence: inferred
- **Wall 4's position in the multibody chain rests on a single model probed through three throwaway repairs, so the claim "the multibody family needs a matcher" is established on n=1 — but the wall itself is independently real.**
  - evidence: docs/ANALYSIS.md final section: "One model, every wall knocked down locally and crudely, nothing kept." Against that, the two Constraints models reach the identical message at HEAD with no hand repairs at all (/tmp/mb-report.txt), and the `why` probe on RevoluteConstraint shows the same two-equation pair.
  - confidence: verified
