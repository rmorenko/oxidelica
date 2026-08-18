# Modelica 3.7 coverage

An honest map of what Oxidelica implements against the
[Modelica Language Specification 3.7](https://specification.modelica.org/maint/3.7/),
chapter by chapter. "Partial" names what is missing; anything not listed
under a chapter works as specified for the subset this project covers.

| Ch. | Area                          | Status  |
| --- | ----------------------------- | ------- |
| 2   | Lexical structure             | Partial |
| 3   | Operators and expressions     | Partial |
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
| 14  | Overloaded operators          | Absent  |
| 15  | Stream connectors             | Absent  |
| 16  | Synchronous language elements | Absent  |
| 17  | State machines                | Absent  |
| 18  | Annotations                   | Partial |
| 19  | Unit expressions              | Partial |

## What works

Flat and hierarchical models; packages with qualified names and imports
(`import A.B;`, `import C = A.B;`); inheritance with modifiers, nested
modifiers down to a child's attribute; `replaceable`/`redeclare` for
components and for classes (`replaceable package Medium = …` with
`constrainedby` checked), conditional components, `inner`/`outer`,
enumerations; connectors with `flow`, connections with subscripts,
inside `for` loops and between whole connector arrays; DAE index
reduction with dynamic state re-selection; events (`when`/`elsewhen`,
`pre`, `edge`, `change`, `initial()`, `sample`, `reinit`, `terminate`,
`noEvent`, `smooth`), event iteration; `initial equation`; runtime
`assert` with its message; functions with several outputs
(`(a, , c) = f(...)` in equations and algorithms), named and defaulted
call arguments (`f(x, precision = 6)`); algorithm
sections in models and functions with `:=`, `if`, `for`, and `while`
with `break` and `return` where the compiler can decide the
conditions; arrays as
values (literals, ranges, slicing with `end`, comprehensions,
elementwise operators, matrix algebra with `transpose`, `identity`,
`diagonal`, `cross`, concatenation via `cat` and `[ , ; ]`, reductions,
`size`, `zeros`/`ones`/`fill`/`linspace`, array-valued functions);
discrete variables and clocked blocks; a static type layer
(Boolean/Integer/Real) and dimensional unit checking (ch. 19), both
permissive: an error needs two declared facts to contradict.

## Known gaps, largest first

**Typing is shallow.** The type and unit layers only reject
contradictions between declarations: a variable with no unit, a call
the checker does not know, or a unit spelled in a symbol outside its
table all pass unexamined, and everything is still carried as a
floating-point number at runtime. `String` exists only as literals in
descriptions and `terminate`; `min`/`max`/`nominal`/`quantity` are
parsed and ignored; unit scale factors are ignored too (`g` and `kg`
are the same dimension, `displayUnit` does nothing); numeric literals
never carry units, so `x = 5` is accepted whatever `x` is declared in.

**Arrays** (ch. 10): no flexible sizes (`:`), no empty arrays, no
`outerProduct`/`symmetric`/`skew`, no Boolean or enumeration
indexing. Dimensions must be compile-time constants.

**Functions** (ch. 12): no recursion, no external C/Fortran, no
`derivative`/`inverse` annotations, no functions as arguments, no
record constructors. Functions are inlined symbolically, which is also
what rules recursion out — and a skipped tuple slot still costs the
work of computing that output's expression. `while` runs at compile
time, so its condition must be decidable there: the trip count cannot
depend on a simulated variable, and a `break` or `return` behind an
undecidable `if` is an error.

**Whole chapters absent**: overloaded operators (14), stream connectors
(15), synchronous clocked constructs (16) — note that `sample()` events
and clocked discrete blocks in the ordinary sense do work — and state
machines (17).

**Structure**: no `expandable connector` or `operator` classes
(`block` parses as a model, causality unchecked);
`protected` is accepted but not enforced; `final` and `each` are parsed
and ignored (array modifiers distribute regardless); package `extends`
does not merge members through an alias; selective model extension
(3.6's `break`) is not parsed. No `MODELICAPATH` or one-class-per-file
directory layout — libraries are flat `.mo` files in `lib/`.

**Semantics**: `der()` only of a plain variable; `if`-equations require
compile-time conditions (the spec also allows run-time ones with
balanced branches); `when` inside algorithms is not supported; vector
`when` conditions are not supported; no `delay`, `terminal`,
`homotopy`, `semiLinear`, `spatialDistribution`, `getInstanceName`;
overconstrained connection
graphs (`Connections.root` and friends) are not implemented; subtype
compatibility (ch. 6) is approximated by the `extends` chain rather
than structural plug-compatibility.
