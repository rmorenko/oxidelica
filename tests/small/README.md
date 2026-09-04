# Small models

One model per fault, as small as the fault allows, named after what it
does rather than after the family it came from.

The rule they exist for: reproduce small before measuring large. Three
shifts went on the corpus at five minutes a run, reasoning about shapes
from the outside, for a fault a twelve-line model showed in a second -
and the model then served as the test. A family of models refusing for
one reason is a signal to write the smallest model that refuses the
same way and work there.

Each file says in its header what it stands for and what the refusal
looks like when the fault is present. A model whose fault is fixed
stays: it is the cheapest guard there is, and `run.sh` reports it.

```sh
tests/small/run.sh          # every model, refusals reported
tests/small/run.sh <name>   # one of them
```

A model here is not a substitute for a test in the suite. When a fault
is fixed, the same case goes into `crates/oxidelica-parser/tests` as
well - the small model is the tool that found it and the cheap check
that it stays found.
