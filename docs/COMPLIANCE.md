# Modelica 3.7 coverage

An honest map of what Oxidelica implements against the
[Modelica Language Specification 3.7](https://specification.modelica.org/maint/3.7/),
chapter by chapter. "Partial" names what is missing; anything not listed
under a chapter works as specified for the subset this project covers.

| Ch. | Area                          | Status  |
| --- | ----------------------------- | ------- |
| 2   | Lexical structure             | Full    |
| 3   | Operators and expressions     | Full    |
| 4   | Classes, types, declarations  | Partial |
| 5   | Scoping, lookup, flattening   | Partial |
| 6   | Type relationships            | Minimal |
| 7   | Inheritance, redeclaration    | Mostly  |
| 8   | Equations                     | Mostly  |
| 9   | Connectors and connections    | Partial |
| 10  | Arrays                        | Partial |
| 11  | Statements and algorithms     | Partial |
| 12  | Functions                     | Partial |
| 13  | Packages                      | Partial |
| 14  | Overloaded operators          | Partial |
| 15  | Stream connectors             | Partial |
| 16  | Synchronous language elements | Partial |
| 17  | State machines                | Partial |
| 18  | Annotations                   | Partial |
| 19  | Unit expressions              | Partial |

## What works

Flat and hierarchical models; packages with qualified names and imports
(`import A.B;`, `import C = A.B;`, `import A.B.*;`, `import A.{B, C};`),
spread over one file or over a
directory tree whose files say with `within` where they sit, found
through `MODELICAPATH`; inheritance with modifiers, nested
modifiers down to a child's attribute, including a whole array handed
to a component (`Chain c(m = {1, 2, 3}, x(start = {0, 1, 2}))`), which
is spread over the elements; `replaceable`/`redeclare` for
components and for classes (`replaceable package Medium = …` with
`constrainedby` checked), conditional components, `inner`/`outer`,
enumerations; connectors with `flow` and `stream`
(`inStream`/`actualStream` with junction mixing), `expandable
connector` buses whose members come from the connections and are
shared with joined buses, connections with
subscripts, inside `for` loops and between whole connector arrays;
DAE index
reduction with dynamic state re-selection; events (`when`/`elsewhen`,
`pre`, `edge`, `change`, `initial()`, `sample`, `reinit`, `terminate`,
`noEvent`, `smooth`, `semiLinear`, `delay`, `nthRoot`, `terminal`,
`getInstanceName`, `spatialDistribution`, `cardinality`), a `when`
watching several
conditions at once and one standing among the statements of an
algorithm; event iteration; `if` equations, structural on a
compile-time condition and balanced-branch on a run-time one — each
mode is matched, torn and solved as its own model and rebuilt when the
condition flips, so branches may constrain different unknowns, which
is how an ideal switch is written;
`initial equation`; runtime
`assert` with its message; functions with several outputs
(`(a, , c) = f(...)` in equations and algorithms), named and defaulted
call arguments (`f(x, precision = 6)`); algorithm
sections in models and functions with `:=`, `if`, `for`, and `while`
with `break` and `return` where the compiler can decide the
conditions; arrays as
values (literals, empty ones, flexible sizes on function inputs with
results shaped by `size(v, 1)`, ranges, slicing with `end`,
comprehensions,
elementwise operators, matrix algebra with `transpose`, `identity`,
`diagonal`, `cross`, concatenation via `cat` and `[ , ; ]`, reductions,
`size`, `zeros`/`ones`/`fill`/`linspace`, array-valued functions);
discrete variables, clocked blocks and periodic `Clock` partitions with
`sample`/`hold`/`previous`/`interval`; state machines of block states
with declared transitions; `operator record` classes whose
arithmetic operators are dispatched on the record they belong to;
a static type layer
(Boolean/Integer/Real) and dimensional unit checking (ch. 19), both
permissive: an error needs two declared facts to contradict; `String`
constants and parameters, built from literals, from each other and
from `String(number)`, compared with `==` and `<>`, and settled before
the run.

Chapter 2 is complete: every reserved word is reserved, quoted
identifiers, the escape sequences, numbers with the digits missing on
either side of the point (`13.`, `.13E2`), non-nesting block comments,
Unicode in strings and comments and 7-bit ASCII in names all follow
the specification. Two of the words are accepted and then ignored -
`pure` and `impure` say how a function behaves, and nothing here acts
on the difference - and `external` is refused by name rather than
honoured, since a function needs a Modelica body.

## Known gaps, largest first

**Typing is shallow.** The type and unit layers only reject
contradictions between declarations: a variable with no unit, a call
the checker does not know, or a unit spelled in a symbol outside its
table all pass unexamined, and everything is still carried as a
floating-point number at runtime. `nominal`, `quantity` and `stateSelect`
are parsed and ignored; unit scale factors are ignored too (`g` and
`kg` are the same dimension, `displayUnit` does nothing); numeric
literals never carry units, so `x = 5` is accepted whatever `x` is
declared in. `min` and `max` are the assertions the specification says
they are: a value settled before the run is refused by the checker,
and one the run produces stops it where it left the bounds. An
`Integer` is refused a value that works out to a fraction, in a
binding, a `start` or an assignment - though `Integer i = 3.0` passes,
since a whole number spelled with a point contradicts nothing.

**Strings** are worked out at the end of flattening and take no part
in a run: the arrays a step works on hold numbers. A `String` constant
or parameter may be built from literals, from another string and from
`String(number)`, joined with `+` and compared with `==` and `<>` -
which is how a model names a medium or a mode and reads it in an `if`.
A comparison becomes the Boolean it comes to and the declaration
disappears. What is missing: a `String` that only a run could produce
(`String(x)` of a variable), string variables in results, the string
functions of ch. 12, and `assert`/`terminate` messages built by
joining - those still take a literal.

**Operators and expressions** (ch. 3): the precedence and the
non-associativity of `^` and of the relations are as written, every
relational operator orders strings the way `strcmp` does, and two
Reals may not be compared for equality - the specification forbids it,
and a run gives no reason to expect a stepped quantity to land on a
value exactly. `terminal()` is true at the stop time of a run that finished, and a
`when` watching it fires there on every solver - a run the model
stopped itself never reaches it, which is the difference between an
analysis that ended and one that succeeded.
`getInstanceName()` answers with the simulated model's name and the
path of the instance that asked, settled before the run like every
other string. `spatialDistribution()` carries a profile along a coordinate rather
than holding a value for a time: what enters at one end leaves at the
other once the coordinate has moved by one, in either direction and
across a reversal, from whatever profile the model started with. It is
exact between output points, since the profile remembers the position
each value entered at rather than sampling a grid. Its arguments but
the two inflows are checked before the run.

`cardinality(c)` answers how many `connect` equations name a port,
which is settled while the connections are still in hand. The
specification deprecates it and says it will be removed in a coming
release, so a model is better off without it - but while it is defined
it is answered, including in the assertion it is nearly always asked
inside.

Chapter 3 is complete.

**Arrays** (ch. 10): no `outerProduct`/`symmetric`/`skew`, no Boolean
or enumeration indexing. A flexible size (`:`) is read from the
argument at the call site, so it belongs to a function input; a model
component still needs a dimension the compiler can work out.

**Functions** (ch. 12): no recursion, no external C/Fortran, no
`derivative`/`inverse` annotations, no functions as arguments, no
record constructors. Functions are inlined symbolically, which is also
what rules recursion out — and a skipped tuple slot still costs the
work of computing that output's expression. `while` runs at compile
time, so its condition must be decidable there: the trip count cannot
depend on a simulated variable, and a `break` or `return` behind an
undecidable `if` is an error.

**Stream connectors** (ch. 15) work in the flat convention: `stream`
variables, `inStream`/`actualStream`, unconnected and pairwise sets,
and flow-weighted junction mixes regularised with a floor of 1e-10.
The inside/outside distinction of 15.2 is not made — connection sets
that bridge a component's boundary port mix with flat signs — and
there is no `positiveMax` with nominal-based regularisation.

**Overloaded operators** (ch. 14) work for the arithmetic: an
`operator record` may declare `'+'`, `'-'`, `'*'`, `'/'` and `'^'`,
either as one function or as a package of them told apart by how many
arguments they take, and the record's own constructor builds it from
its fields in order. What is missing: `'constructor'` and
`'0'` are not consulted, comparison and `String` operators are not
dispatched, and the record a value belongs to is worked out from the
expression rather than from a type of its own — an operator whose
result is a different record than its operands would be misread.

**Synchronous elements** (ch. 16) cover the periodic case: a
`Clock(interval)` declared with an interval the compiler can work out,
`sample(u, c)`, `hold(u)`, `previous(x)` and `interval(c)`. Which
equations belong to a clock is inferred and spreads from a sampled
value to whatever reads it; what cannot be on a clock — a derivative,
say — must ask for the held value by name, and says so otherwise.
What is missing: event clocks (`Clock(condition, …)`), rational
clocks (`Clock(counter, resolution)`), `subSample`/`superSample`/
`shiftSample`/`backSample`/`noClock`, `solverMethod` partitions that
integrate inside a clock, and clocks as arguments or members.

**State machines** (ch. 17) are built on the clock: states are block
instances, `initialState` and `transition` declare the graph, and
`activeState`, `ticksInState` and `timeInState` answer from the
machine's own bookkeeping. A state's equations count only while it is
in force, the rest hold what they had, and an arrival puts a state's
variables back to their start values as `reset = true` asks.
What is missing, and it matters: a transition is judged on the values
from the tick before and takes effect at the next tick, so what the
spec calls `immediate = true` — the default — behaves as
`immediate = false` does. There is one machine to a model, running on
its one clock; `synchronize` and hierarchical states are not
supported, and a `reset` is taken per state rather than per arrow.

**Structure**: no `operator` classes (`block` parses as a model,
causality unchecked); an `expandable connector` takes each member's
type from the other side of the connection that names it, so a member
connected only to another bus member has nowhere to get one, and
`each`-style array members of a bus are not supported;
`protected` is accepted but not enforced; `final` and `each` are parsed
and ignored (array modifiers distribute regardless); package `extends`
does not merge members through an alias; selective model extension
(3.6's `break`) is not parsed. A library may be a directory tree, but
a file's place in it comes from its `within` clause rather than from
where the file sits, so a tree without `within` headers is read as
though it were flat. An unqualified import reaches classes but not
constants: after `import A.*;` a bare `pi` is taken for a variable of
the model, and `import A.pi;` is the way to write it.

**Semantics**: `der()` only of a plain variable; a condition of a
run-time `if` equation must be readable from the parameters, the
states and plain `name = expr` definitions at the instant the mode is
settled — a condition that only an algebraic loop could produce falls
back to the `else`. A `when` may stand among the statements of a
model's algorithm section, but only at the top of one and holding
whole-variable assignments. `homotopy` takes the real problem and goes
straight at it, which the specification permits and which means no
continuation is run. `Connections.root`, `potentialRoot`, `branch`,
`isRoot` and `rooted` decide the roots of an overconstrained graph and
refuse one with no root or with two, but no equality constraints are
generated or dropped, since overdetermined types are not supported.
`delay(u, T)` keeps what `u` was at each output point and reads
between them in a straight line, so the shift is as exact as
`Interval` is fine and the step is never longer than the delay; `T`
must be known before the run, and a continuation after a
re-compilation starts its memory over. No `terminal`,
`spatialDistribution`, `getInstanceName`; subtype
compatibility (ch. 6) is approximated by the `extends` chain for
`constrainedby`, though a connection set matches its connectors by
shape — same member names, same `flow` and `stream` prefixes — rather
than by class name.
