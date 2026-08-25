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

## Language

Everything in the repository is English - code, comments, commit
messages, documentation - except files ending `.ru.md` and the Russian
locale. `scripts/check_cyrillic.py` enforces it and runs in CI.

Commit messages are prose, and they explain why the change was worth
making and what was wrong before. Not bullet lists, and no mention of
the tools that happened to be involved.
