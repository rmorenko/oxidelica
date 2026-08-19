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
| 7   | Inheritance, redeclaration    | Full    |
| 8   | Equations                     | Mostly  |
| 9   | Connectors and connections    | Partial |
| 10  | Arrays                        | Full    |
| 11  | Statements and algorithms     | Partial |
| 12  | Functions                     | Partial |
| 13  | Packages                      | Full    |
| 14  | Overloaded operators          | Full    |
| 15  | Stream connectors             | Full    |
| 16  | Synchronous language elements | Full    |
| 17  | State machines                | Full    |
| 18  | Annotations                   | Mostly  |
| 19  | Unit expressions              | Full    |

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
conditions — a loop runs over a range, a stepped range, a set or an
array alike, takes several indices at once (`for i in 1:2, j in 1:3`),
and takes none at all (`for i loop`), reading the range off the array
the body subscripts by it; `assert` may be written among the statements
of a model, where inside a loop it is one check per round; arrays as
values (literals, empty ones, flexible sizes read from the value a component or a function input
is given, with results shaped by `size(v, 1)`, ranges, slicing with `end`,
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
on the difference. `external` is read so that a file holding one such
function alongside others still loads, and refused where such a
function is called; the exception is `external "builtin" y = asin(u)`,
which says the function is the operator the language already has, and
a call to it becomes a call to the operator.

## Known gaps, largest first

**Typing is shallow.** The type and unit layers only reject
contradictions between declarations: a variable with no unit, a call
the checker does not know, or a unit spelled in a symbol outside its
table all pass unexamined, and everything is still carried as a
floating-point number at runtime. `nominal`, `quantity` and `stateSelect`
are parsed and ignored; numeric literals never carry units, so `x = 5`
is accepted whatever `x` is declared in. `min` and `max` are the assertions the specification says
they are: a value settled before the run is refused by the checker,
and one the run produces stops it where it left the bounds. An
`Integer` is refused a value that works out to a fraction, in a
binding, a `start` or an assignment - though `Integer i = 3.0` passes,
since a whole number spelled with a point contradicts nothing.

**Inheritance** (ch. 7) is complete: a class extends its bases with
their modifiers and redeclarations, a `final` declaration is closed to
modification from an enclosing class, `each` spreads a modifier over an
array where a bare list is handed out one entry per element, and a
selective `extends Base(break f)` leaves a component and its
connections out - `break connect(a, b)` one connection - refusing a
break that matches nothing.

**Packages** (ch. 13) hold classes and constants and nothing else - a
parameter or a variable in one is refused, since a package has no
instance to own a value. The imports are all four forms (`import A.B;`,
`import C = A.B;`, `import A.B.*;`, `import A.{C, D};`), a wildcard
reaching a package's constants as well as its classes but outranked by
a component of the model with the same name; `extends` merges a base
package's constants into the derived one, later declarations winning;
and `encapsulated` is a wall a simple name inside a package does not
reach past, so it must be imported or built in - the overloads under a
quoted operator symbol excepted, since they exist to serve their
record and still see it.

**Unit expressions** (ch. 19) follow the grammar in full: the SI base
and derived units with every prefix, the non-SI units the chapter names
(`min`, `h`, `d`, `l`/`L`, `eV`, `deg`, `bar`), a single division with
a parenthesised compound denominator, and rational exponents written
`m(1/2)`, which the dimensions are kept as fractions to hold exactly -
so `sqrt` of a length is `m(1/2)` and squaring it gives the length
back. The one thing dropped is the scale factor, on purpose: `g` and
`kg` are the same dimension and `displayUnit` does nothing, which is
exactly right for checking consistency and says nothing about a value.

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

**Arrays** (ch. 10) are complete: literals, ranges, slicing with `end`,
comprehensions, elementwise operators and the matrix algebra
(`transpose`, `identity`, `diagonal`, `cross`, `outerProduct`,
`symmetric`, `skew`, `cat` and `[ , ; ]`), the reductions and the
constructors. A dimension may be a number, a type - `Real x[Boolean]`
has two elements indexed off `false`, `Real x[E]` one per enumeration
literal - or a `:` that reads its length from the value the component
is given.

**Functions** (ch. 12): no external C/Fortran bodies (they are read and
refused where called), no functions as arguments, no record
constructors. A function is inlined symbolically
wherever it can be, which is what lets the compiler differentiate
through one and fold it away where the arguments are known — and a
skipped tuple slot still costs the work of computing that output's
expression.

Where inlining cannot reach, the call is left standing and the run
walks the body for itself, one number at a time. Two things cannot be
inlined: a function that leads back to itself, directly or through
others, which has no bottom to unroll to; and a `while` whose trip
count the model decides rather than the compiler. So recursion and a
data-dependent loop both run, and `5!` written the obvious way comes
out at 120.

What a walked body may hold is narrower than what an inlined one may,
because a walk carries numbers: an array or a `String` inside one is
refused, as is a body giving more than one answer, and a `when` has no
meaning there since there is no event inside a call. A walk that will
not end is stopped and named — 64 calls deep, or ten million rounds of
one loop. And a walked body is opaque to everything the symbolic layer
does: it cannot be differentiated except through a `derivative`
annotation, and it is not folded where its arguments are known.

Among a model's own statements a `while` is still unrolled where it
stands, so its trip count has to be settled there: only a call can be
left for the run to walk.

A function may say how to differentiate itself:
`annotation(derivative = f_der)` names a function taking what this one
takes and then one derivative for each, and the call is inlined for its
value while keeping that rule beside it. Differentiation reaches for
the rule instead of taking the body apart, which is what lets a body
the differentiator cannot read — one with `abs` in it — still carry a
model that needs its constraint differentiated or its Jacobian built.
The options the specification allows beside it (an order, a
`noDerivative`, a `zeroDerivative`) change which arguments the named
function takes and what it answers, so an annotation carrying one is
read past and not kept: the call stands with no derivative rule of its
own rather than a wrong one, and a derivative asked of it is refused by
name. Reading one wrong would give a wrong derivative and nothing
downstream could catch it. `annotation(inverse(x = f_inv(y)))` is read
and checked — the function has to exist, the input has to be one this
one takes, the arguments have to be things it has to hand — and then
set aside: the nonlinear corrector already solves `f(x) = u` for `x`,
so an inverse would save work rather than make anything possible.

**Stream connectors** (ch. 15) are complete: `stream` variables - one
`flow` beside them, as the chapter requires - `inStream` and
`actualStream`, unconnected ports, pairwise sets and junctions of any
size. A junction weighs each port by what it pushes into the node,
`max(-m, 0)`, so a port pushing nothing has no say in the mix; the
regularising floor of 1e-10 is in the divisor alone, which is what
`positiveMax` is for. Which way a port pushes depends on the side of
its class it sits on: a port of the class the connection is written in
has its flow entering the node where a component's port has it
leaving, the sign convention of 9.1.2. The one thing dropped is
regularisation scaled by `nominal`, which follows the shallow typing
above - `nominal` is parsed and ignored everywhere.

**Overloaded operators** (ch. 14) are complete: an `operator record`
may declare `'+'`, `'-'`, `'*'`, `'/'`, `'^'`, the six relational
operators, `'constructor'`, `'0'` and `'String'`, each either as one
function or as a package of them told apart by how many arguments they
take. A constructor is consulted before the default field-order one; a
comparison of records dispatches and returns a Boolean; `String(a)` is
what the record's own `'String'` makes of it; and `sum` over an array
of records adds them with `'+'` starting from `'0'`, or from the first
element where no `'0'` is declared. An operator function may build its
record output field by field. The record a value belongs to is still
worked out from the expression rather than from a type of its own, so
an operator whose result is a different record than its operands would
be misread.

**Synchronous elements** (ch. 16) have all four constructors and all
the conversions. A clock is `Clock(interval)`, the exact fraction
`Clock(counter, resolution)`, the event clock `Clock(condition,
startInterval)`, or one of those carrying a solver method. Which
equations belong to a clock is inferred and spreads from a sampled
value to whatever reads it; `sample`, `hold`, `previous`, `interval`
and `firstTick` say what they say, and what cannot be on a clock must
ask for the held value by name.

A derived clock is kept as its root plus two exact fractions — the rate
and the shift — so `subSample`, `superSample`, `shiftSample` and
`backSample` are arithmetic on the fractions rather than on seconds,
and a round trip lands back on the clock it started from instead of a
rounding away from it. `noClock` reads a clocked value without
inferring its clock. Partitions are ordered by what they read, so two
clocks ticking at the same instant compute in an order where each has
what it needs; a pair that needs each other within one tick is refused,
as is a value written on two clocks at once.

An event clock's `interval` is measured — the time now less the time at
the tick before — and its first tick answers with the start interval
the constructor was given. It may be sub-sampled, which counts rising
edges; the other three conversions ask where a tick falls between two
others, which 16.5 forbids on an event clock and which nothing could
answer anyway.

`Clock(c, solverMethod)` steps a `der` across the ticks with the
tableau of the named method: `ExplicitEuler`, `ExplicitMidPoint2` and
`ExplicitRungeKutta4` are worked, and an event clock takes the
one-stage method only, the others needing a step known before it is
taken. `ImplicitEuler`, `ImplicitTrapezoid` and `External` are refused
by name — an implicit method makes every tick an equation to solve
where a tick here is a list of assignments, which is the wall ch. 11
stands behind. The specification asks a tool to spell the methods it
works the way it spells them, not to work them all.

A clock or a factor may be left unsaid. An equation is on one clock, so
where it names a clock that knows its rate beside one that does not,
the second takes the first: `Clock()` and `Clock(0, resolution)` become
the clock they meet, and a `subSample` or `superSample` with no factor —
or with the zero that means the same — finds it from the two rates. The
factor is worked back out and checked rather than divided out and
trusted, so a ratio that merely rounds to a whole number is refused,
with the factor it would have taken named. `Clock(0, resolution)` keeps
the denominator it was given, so what turns up for it has to be a whole
number of those. Nothing left to work a clock out from is refused too:
an unsettled clock would have nothing lifted onto it, and the equations
meant to tick would quietly stay continuous.

Chapter 16 is complete.

**State machines** (ch. 17) are built on the clock: states are block
instances, `initialState` and `transition` declare the graph, and
`activeState`, `ticksInState` and `timeInState` answer from the
machine's own bookkeeping — `timeInState` counting periods, so a
machine on an event clock is refused it and keeps `ticksInState`. A
state's equations count only while it is in force, the rest hold what
they had, and an arrival puts a state's variables back to their start
values as `reset = true` asks.
A transition is judged on this tick's condition and the state it names
takes over at the next tick, which is what 17.3.4 calls
`immediate = true` — the default; `immediate = false` keeps the answer
for a tick and is taken on what it kept, so everything happens one tick
later. `reset` belongs to the arrow rather than to the state it arrives
at, so a state reached by two arrows starts over by the one that asked
and carries on by the one that did not. Two arrows out of one state
saying the same thing about which goes first are refused, as the
chapter requires.

A model may hold several machines, and a state of one may hold others:
the arrows say which, since a machine is a set of states joined by
arrows and nothing joins one machine to another. A machine whose states
all live under one state of another is inside it, and runs only while
that state is in force — nowhere at all before its first arrival,
starting over where the arrow that reached it asked, and keeping where
it got to after the state is left. That is 17.3.3's `active` input, and
it is the whole of what makes a machine hierarchical. `synchronize`
follows from it: the arrow waits until every machine inside the state
it leaves has reached one no arrow leaves, and asks about the tick
before — asking about this one would be asking the machine inside about
an answer that waits on the machine outside, which waits on it.

A variable declared outside the states and written by several of them
is one definition of it (17.3.5): whichever state is in force has its
say, and where none does the variable keeps what it held - which is
what `last()` says there and what the value from the tick before says
here. Writing it both inside a state and outside every state is
refused, since a variable has one definition. Every equation carries
the instance it was written inside, which is what tells one state's say
from another's when what they define lives outside them both.

Chapter 17 is complete.

**Annotations** (ch. 18) are read rather than stepped over. An
annotation is a tree of `name = value` where a value is an expression -
a number, a string, a list, a call with named arguments - which is what
the expression parser already reads, so that is what is kept: the
drawing of an `Icon`, a `Documentation`, a `Dialog` group, a `version`.
They travel with the class and with the declaration, and `class_info`
hands the class's out to whatever draws it. What the parser cannot read
is stepped over rather than refused, and the rest of the annotation is
still read - an annotation says things to tools, and one a tool does
not understand must not stop it.

Everything the chapter calls an error is one. `mustBeConnected` refuses
a port nothing connects to and `mayOnlyConnectOnce` a port named twice,
each with the message its declaration wrote; `Evaluate = true` refuses
a parameter whose value the compiler cannot settle, since asking to be
evaluated and not being is not a thing to pass over. `experiment` says
how long a run is and how often it writes, and `HideResult` leaves a
variable out of the results while changing nothing else about it -
which the chapter leaves to the tool and this one does.

What is left is not the language's half but the tool's, and it is one
thing said three ways: the chapter requires a tool to draw `Icon`,
`Diagram` and `Placement`, to build a dialog from `Dialog`, `choices`
and `connectorSizing`, and to enforce `Protection`. All three are read
and carried and reachable - `class_info` hands a class's annotations to
whatever draws it - and none is acted on, because what would act on
them is the editor rather than the compiler. The editor draws symbols
written by hand today. `uses` and `conversion` are the same shape:
there is nothing to convert between until libraries are loaded by
version. `Inline`, `LateInline`, `smoothOrder`, `singleInstance` and
`TestCase` the chapter leaves to the tool, and this one does not act on
them.

**The standard library** is the measure this map is checked against:
of the Modelica Standard Library's 2674 files, 2628 parse, and 209 of
its 670 example models flatten. What stands in the way of the rest is
measured rather than guessed - `oxidelica library check` ranks the
reasons - and the list is in [MSL.md](MSL.md). The largest of them:
a class of `Blocks.Math` that is still not read, the multibody
library's arrays, a clock whose interval comes from a count of ticks,
and external C bodies.

**Structure**: no `operator` classes (`block` parses as a model,
causality unchecked); an `expandable connector` takes each member's
type from the other side of the connection that names it, so a member
connected only to another bus member has nowhere to get one, and
`each`-style array members of a bus are not supported;
`protected` is accepted but not enforced. A library may be a directory tree, but
a file's place in it comes from its `within` clause rather than from
where the file sits, so a tree without `within` headers is read as
though it were flat.

A call whose body is written outside Modelica and which answers with
nothing is read and dropped: `Streams.print(...)` writes a line on a
terminal, there is none here, and no value goes missing. One that does
answer is refused where it is called, its value being wanted.

**Semantics**: `StateSelect` is the language's own enumeration and a
model may declare with it and read its literals, but what it asks for
is not acted on: `never` and `always` are demands about which variables
are integrated, and the states here are chosen by where `der` appears
and by index reduction. `der()` only of a plain variable; a condition of a
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
re-compilation starts its memory over. Subtype
compatibility (ch. 6) is approximated by the `extends` chain for
`constrainedby`, though a connection set matches its connectors by
shape — same member names, same `flow` and `stream` prefixes — rather
than by class name.
