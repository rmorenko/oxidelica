# Working on Oxidelica

Notes for anyone, human or otherwise, picking the project up. What the
project _is_ belongs in [README.md](README.md) and
[docs/CONCEPT.md](docs/CONCEPT.md); this is about how work on it is
done, and mostly it is the things that were learned by getting them
wrong.

## What counts as progress

The measure is how much of the Modelica Standard Library this compiler
can actually run, not how much of the specification is nominally
covered. `scripts/library_floor.sh <library>` prints three numbers -
files read, models flattened, models run - and holds them against
floors it will not let drop. Quote the numbers before and after a
change; a change that moves none of them should say why it was worth
making anyway.

A commit that moves the numbers moves the floors in the same commit.
This was learned the hard way: twenty-three models were won over a
session and the floors left where they were, so a regression giving
all twenty-three back would have passed CI without a word. A ratchet
that is not wound is not a ratchet. There are five floors and the
runnable pair is easy to forget - take all five from one run of the
script rather than from what the commit messages said.

A rule that is right everywhere beats one that is right almost
everywhere, even where the second is what other tools do. `abs` under
a derivative could have taken `sign(x)*der(x)`, which is wrong at
exactly zero - and zero is not a corner physical models avoid, it is
where they work: a relay switching, friction breaking away, a gap
closing, a flow reversing. The rule taken instead says nothing about
`abs`: whatever does not move has a derivative of zero. Narrower,
true everywhere, and it generalises past the case that prompted it.

A name shortened to its tail is a guess, not a resolution. Twice now
the same fault: a namesake of `fill` took the built-in's shape, and a
derivative rule was matched by the last part of a dotted name, which
would have given anybody's own `sin` the built-in's derivative. Both
give a wrong number where a refusal was owed, and a wrong number is
the worst thing this compiler can do. What resolution looks like here
is an empty head - `.sin`, the compiler's own function with its path
resolved away - and anything with a path still on it belongs to
whoever wrote it. If a name has to be shortened to be recognised, the
lookup is in the wrong place.

Two numbers are comparable only if the same binary produced them. A
lookup was recorded as firing nowhere, then as costing thirteen
models: the difference was two builds mistaken for one. Put the change
behind an environment switch and run the corpus twice from one binary,
so that the only thing that differs between the numbers is the thing
being measured.

Reproduce small before measuring large. Three shifts went on the
corpus at five minutes a run, reasoning about shapes from the outside,
for a fault that a twelve-line model showed in one second - and the
small model then served as the test. If a family of models refuses for
one reason, write the smallest model that refuses the same way and
work there.

`oxidelica library check <library>` ranks what the library stumbles
over, commonest first. That ranking is the work queue. Take the top of
it, not whatever is most interesting.

## Before pushing

`scripts/preflight.sh` runs on one machine what the six CI jobs run
between them, and stops the round trip of pushing, waiting several
minutes and finding out a linter had something to say. `--quick` is
the fast half while a change is still moving. CI is _six_ jobs: the
tests on three platforms, the library floor, the advisories, and a
coverage and linters job holding a 95% line threshold that is easy to
forget until it turns a commit red.

Coverage is measured by `scripts/coverage.sh --summary-only`. Write
its output to a file rather than piping it to `tail`, which eats it.

`cargo build` and `cargo test` passing is not the same as CI passing,
and the difference is the linters and the coverage floor. Clippy's
lints are warnings on a local build and errors under CI's `-D
warnings`, so a change can be green on the desk and red three commits
later on the server - which is exactly what a refactor does, since
turning owned locals into borrowed arguments leaves `&x` where `x` is
now wanted, sixty times over. The coverage floor catches the other
half: three hundred new lines with two tests over them will hold every
threshold this project measures and still turn the build red.

Both are in `scripts/preflight.sh`, along with the formatters that
check the prose. Run it before pushing rather than after; the round
trip of ten red commits was paid for once already, and three more went
red for a note whose emphasis was written with the wrong character.

Editing a note counts. The Markdown is held to a style, and a commit
that only touches `AGENTS.md` can turn the build red exactly as a
commit touching the compiler can.

## Finding out why a model will not run

`oxidelica why <model> <variable>` says where a variable's value was
meant to come from: the declaration, the binding, the start attribute,
every equation and `when` naming it, and what the compiler in the end
made of it. It takes a file or a class of the libraries, so a
standard-library model can be asked about directly. A record is
answered field by field, which is where values usually go missing.

This replaced the routine of writing a throwaway model, adding
`eprintln!` to the flattener, rebuilding, and taking it all out again.
Where it does not answer the question, the fix is usually to make it
answer the question rather than to go back to the printlns.

Shrink the failing model rather than growing a synthetic one. A
synthetic model tests the layer you already imagined: seven of them
were written against a refusal about redeclared media, all seven
passed, and the bug turned out to be in the record builder, where no
redeclaration layer goes. Throwing components out of the real model
while the refusal survives leads to the layer that could not be
imagined.

`OXIDELICA_WHERE=1` puts the file and line on a compiler refusal, which
answers which of the several places raising a message raised this one.
A debugger was measured against this and lost: it has no Rust plugin
here, so `String` and `Expr` read as addresses. It answers "where" and
not "what", and "where" is the cheaper half to build in.

## Refusals

A refusal must name what it refused. "Subscripts survive flattening
only as scalars" hid a named argument for weeks; saying _which_
construction was found is what uncovered it. The same went for the
parameters a model could not evaluate: a list printed in the
compiler's inner spelling of the expressions said far less than naming
the one variable nothing gives a value to.

Guessing is worse than refusing. A value quietly defaulted to zero is
a wrong answer presented as a right one, and every one of those found
so far had been silently wrong for a long time.

## A census entry is not a family

The run half's census is read by counting kinds, and a kind that
counts high is not therefore one cause. Measured on the library at
733 flatten / 334 run: 71 models refused as unbalanced, of which the
machine-shaped ones - `airGap`, `pin_ap`, `fire_n` - are 13. The
machine chain and the census's top entry are not the same family, and
work aimed at the top of the list would not have reached the machines.

The reverse also happens: a whole entry can be emptied without a model
moving. The nine `$initial` refusals were one cause, one line, and the
entry is now absent from the census - and the nine models still do not
run, because behind the refusal that named the compiler's own name
stood an algebraic loop that diverges. A kind removed is worth
recording as a kind removed, and the run count is a separate claim
that needs its own number.

The general shape: the census counts refusals, and one model has one
refusal at a time - the first one it meets. Emptying an entry uncovers
whatever stood behind it, which may be a wall of its own. Expect the
first number to move only when the last wall in a model's way falls,
and say which of the two a change was.

## Measuring a barrier

A barrier is a pair of numbers from one pipe: how many models stood at
it before a change, and how many stand at it after. `scripts/refusals.sh`
is that pipe, and it prints both halves - the models that would not
flatten and the models that flattened and would not run - because a
plan made from one half alone is made half blind.

A zero counts only where the same pipe can print something other than
zero. This rule is written because of what happened without it: a
barrier was measured for three rounds of work with
`./scripts/why_not.sh`, a script that does not exist. The shell said
so on stderr, the measuring command sent stderr to `/dev/null`, `grep
-c` counted an empty input, and `0` was printed and read as "the
barrier is gone". It had not moved at all: the true count was the same
before and after, and three rounds of work were built on the reading.

So: a measuring pipe does not silence stderr. A script lives in
`scripts/` under `set -euo pipefail`, where a missing file is a
failure and not a zero. And before a number is believed to have fallen
to nothing, the same command is run on unchanged code to see it print
the number it is supposed to have removed.

### A chain is taken whole or not at all

A probe sometimes shows that one barrier stands on another, and that
one on a third: remove the first and the model dies one step later,
in the same family, for the same underlying reason. Four such links
were found in an afternoon on a single parameter of a water tank.

Do not take such a chain apart a link at a time, measuring after each
and reverting what measures zero. A link removed from the middle of a
chain moves no number by construction - the model still dies, one
step further along - and reverting it throws away the only work that
was actually done.

Instead: walk the chain to its end with the probe first, keeping each
removal local and unrecorded, until the model either runs or reaches
something that is not the same family. Write the list of links down.
Then take the chain as one series, with the thresholds moved once at
its end, and the numbers in the message the numbers of the whole
chain.

A zero on a link of a mapped chain is not grounds for reverting the
link. What is grounds for reverting is a chain that was not walked to
its end - because then nobody knows whether it has one.

### A list of work is finished, not sampled

Where the work arrives as a numbered list, every number is answered
before the shift is reported: taken with its number, or measured and
written down with what it cost, or named as parked with the reason.
A pause in the middle to report progress is not the end of the list -
the next thing after reporting is the next number.

What is never done is picking the interesting ones. The dull number
is where the surprise usually is, and a list half-done is a list
whose remainder nobody can plan against.

A chain deeper than five links takes its map to a second reader. Not
because the walk is beyond one pair of hands: because ten links held
in one head are ten chances at the same blind spot, and by the fifth
the map is already written - which is the expensive half of asking.
The map goes as it is: every link numbered, what was taken and on
what number, what was parked and why, and the last measurement made
rather than the last guess.

## Counting kinds, and what the count cannot see

`refusals.sh` counts kinds of refusal, and the ranking it prints is
the work queue. It has a blind spot worth stating outright: it reads
the _text_ of a refusal, and a probe reads the _layer_ the refusal
comes from. One family whose message quotes whichever parameter came
first is ten kinds to the counter and one layer to the probe.

So: the count of kinds is a lower bound on the number of families,
and probing is the upper one. The gap between them is the work. A
half of the register that looks like nothing but singles may be one
family that has not been probed yet - which is exactly what the first
batch of singles found, ten notes on one layer.

Which leaves the method for singles standing but narrowed: it is for
where probing has actually put models in different layers, not for
wherever the counter shows a column of ones. Probe first, then
decide which method the batch wants.

## The register is an instrument, not a report

`refusals.sh` costs five and a half minutes now, which changes what it
is for. It used to be a thing done once a stage, to decide what to work
on next. It is cheap enough to be a measurement.

Two uses follow, and both were learned by doing them:

**Before and after a series, to see a family move.** Two adjacent
censuses say what one cannot: taking the derivative's input count made
the `derivative takes N inputs` row (9) vanish and the `arrays of N and
M do not fit` row go from 8 to 17. The same nine models, one floor up.
No count of models could have shown that, and neither could reading the
diff.

**After a fix, to see where a family went.** A repair that takes a
family one storey up moves no count at all - the models still refuse,
at the next wall. The register is the only instrument that shows it:
the row that named the old wall empties, and another fills by the same
number. That is how nine `nXi` models were followed from a derivative's
input count to `arrays of N and M do not fit`. Without it, a shift's
work on a chain looks like a shift with nothing to show.

**Before and after a pure move, to prove it was one.** A refactor that
moves code and changes nothing must leave the register identical line
for line. If a row moves, the move was not pure - something was
improved or broken on the way, and the difference is the finding. This
is a stronger check than the tests, which only know what someone
thought to ask about; the register asks the whole library at once.

**Against a reading of itself, because a kind is a row and not a
family.** Two rows may carry the same words: the register holds both
`unknown variable X` and `unknown variable X in equation`. Read as one
row, the count went 21 to 41 and twenty models looked new. Added
together, the two went 45 to 48 - a growth of three, the rest being
one row draining into the other as models travel past a wall. A shift
that had read the single row would have gone looking for seventeen
models that never existed.

So the rule before choosing work from the register: add the rows that
mean the same thing, and only then read the number. The counter splits
a family by its wording, and the wording is not the family - which is
the same blind spot named above, seen now from the other side. It is
worth stating twice because the first statement did not stop it.

## A cancelled run is not a passed run

CI's verdict is read from a run that finished, not from the absence of
a red mark. A cancelled or superseded run leaves the same blank space
on the page as a green one, and a commit pushed on that reading has
been checked by nobody.

`scripts/preflight.sh` before pushing is what makes this cheap to
live with: the local answer is known before the remote one is asked
for, so a cancelled run means the next push covers it rather than
meaning something has to be reconstructed.

## When a change makes the compiler slow

Two scars, both paid for in whole shifts.

**A cache trims repeats, not a giant.** The first reach for a slow
path is a table of answers, and it is usually the wrong one. A cache
of _misses_ is worse than useless: it remembers an entry for every
name a library ever writes, which is unbounded, and still pays a hash
on each. Cache hits over a bounded key, or do not cache.

**Sort the tests by price, and judge cheaply first.** The giant that
cost a shift was neither a missing gate nor a missing cache: it was
the order of the tests. Minting a medium's constant asked for the
value - which gathers a whole package - before asking whether the
declaration had a unit at all, which is one comparison over one
class. `T_default` has no unit, was asked forty thousand times over
one model, and paid the full price each time to hear a cheap "no".
The same rule as the refusal that costs nothing: a test that can say
no for free says it first.

And the stopping rule for either: the library check returns to its
usual eleven minutes, and the counts outside the layer that changed
are identical to the digit. A change that slows models with nothing
to do with it changed the hot path for everybody, whatever it was
meant to touch. Narrow the pipe while hunting - a handful of models
from the chain plus a handful of controls, timed one by one - and
spend the full pass once, at the close. Ten forty-minute passes are a
shift.

**Before reaching for a table, ask what makes two askings
different - and then what holds it still.** This is the general form
of the other two, and it was earned three separate times. The wrong
first question is "how do I remember this"; the right one is "which
parts of the world does this answer depend on", and after that "over
what stretch of the run are those parts guaranteed not to move".
This compiler already had three answers to the second question -
`Inlined::open` over one class's instantiation, `StandingNames` over
the life of a registry, `WALKED` cleared with it - and the array
layer's table became simple only once it was given the first of them
as a bracket. A key built without a bracket must name the whole
world; a key inside one names what the bracket does not hold still,
which is usually two things and not twenty.

And the tests are the judge of it, not the clock. A key that was
wrong three times running was called wrong by three different tests,
each naming a dependency the key had missed - the medium on the
mark, the shapes handed in from a class above. A red test on a
performance change is the cheapest possible news.

The same order was got wrong three more times in one shift, all in
the constants layer, and each cost the whole pass: a binding
substituted whole before asking whether it could answer at all; a
package's entire basket substituted before the fixpoint that settles
nearly every constant without substitution; a ledger that remembered
its successes and not its refusals, so every name that was not a
candidate did the dear work again to say so. Eleven minutes to fifty,
three times, one cause.

There is a known one waiting: `DoublePendulum` takes forty-five
seconds on its own, and has for as long as anyone has measured. When
the pass time next presses the ceiling, that is where to start - one
model eating more than most families. The reference numbers for that
judgment: the whole corpus is eleven minutes over 1043 models, and
CI's library job runs in forty-six against a ninety-minute ceiling.
One model taking minutes is a giant, not work that was bought.

## Tests

A bug fix comes with a test that fails without it. Check that it does:
undo the fix, watch the test go red, put it back. A test written after
a fix and never seen to fail has not been shown to test anything.

Prefer a test that checks a number came out right over one that checks
a model flattened. Flattening is not evidence that anything is
correct.

## Asking fable

Some walls are not worth ramming alone. When a cause has survived a
few honest attempts at a local fix - each attempt moving the failure
rather than removing it - that is the sign the question is
architectural, and there is a standing arrangement for those: a
consultation with a stronger model that reads this repository.

The convention is a document, `QUESTION_FOR_FABLE.md` at the
repository root. Append a question to it - the symptom, a reproduction
small enough to quote whole, the trace that establishes the cause, and
what was already tried - then run `scripts/ask_fable.sh`. The script
hands the document to a fresh session of the consulting model, which
studies the code the question names and appends its answer to the same
document. The document is the memory: it carries every earlier
question and answer, so append to it rather than rewriting it, and a
later consultation can build on an earlier one.

Facts before opinions, in the question as everywhere: a quoted trace
is worth more than a paragraph of suspicion, and listing the fixes
already tried saves the answer from recommending them. The document
and its reproductions are scratch, kept out of the repository by
`.gitignore` - which is also what lets them be written in Russian.
Each consultation is a full session on the shared subscription, so it
is for being stuck, not for code review.

## Refactoring

Refactor by friction, not by schedule. The planned rounds are over;
what remains is to cut when the code says it is time, which it does in
three ways:

- a fix had to be threaded through three or more places;
- what you needed was not findable in a file inside a minute;
- a new kind of silent mistake turned up - a walk that skipped a node,
  a catch-all that swallowed a case.

Any of those is the signal. Cut then and there, in the middle of the
feature work, rather than writing it down for a round that may not
come: one step per commit, the thresholds unmoved, the preflight
before the push, exactly as the rounds did it.

Cutting because a file is long is not on the list. Several files
carved this way turned out to need nothing - `check.rs` was already
one function per check, and the component loop in `instantiate` reads
eleven things its neighbours worked out, so naming it would have moved
the names rather than explained them. Say so and move on; a stage that
was considered and left is worth a sentence saying it was considered.

## Walking an expression

`Expr::map_children` applies a function to each expression one level
down, and `try_map_children` does it for a walk that may refuse. Nearly
every pass is one interesting case and then the same twenty lines of
taking every variant apart and putting it back together; written out
by hand, a variant added to `Expr` is a variant that pass quietly walks
past, which no compiler can see.

Use them for the machinery only. Several walks in this repository stop
at the variants they do not name - they read a subscript and not what
it subscripts, or a call and not a matrix - and that is deliberate,
so they go on saying it themselves. The helper is for the walk that
means "and the same, further down".

A pass that has to _decide_ about a node is another matter, and there
a catch-all is a trap: `const_eval` answering `None`, `shape_of`
naming the node, `to_scalar` reducing it. A variant added to `Expr`
then joins the catch-all silently, and what that looks like from
outside is a dimension that cannot be measured or a value that cannot
be carried, three layers further on. Those match every variant by
name, so the next one has to be decided about rather than absorbed.
A `matches!` asking whether a node is _one of a set_ - is this a
Boolean, is this a list - is not one of these: the question there is
membership, and the catch-all is the answer.

## Language

Everything in the repository is English - code, comments, commit
messages, documentation - except files ending `.ru.md` and the Russian
locale. `scripts/check_cyrillic.py` enforces it and runs in CI.

Commit messages are prose, and they explain why the change was worth
making and what was wrong before. Not bullet lists, and no mention of
the tools that happened to be involved.
