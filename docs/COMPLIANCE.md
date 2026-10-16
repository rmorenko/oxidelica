# Modelica 3.7 coverage

An honest map of what Oxidelica implements against the
[Modelica Language Specification 3.7](https://specification.modelica.org/maint/3.7/),
chapter by chapter. "Partial" names what is missing; anything not listed
under a chapter works as specified for the subset this project covers.

| Ch. | Area                          | Status  |
| --- | ----------------------------- | ------- |
| 2   | Lexical structure             | Full    |
| 3   | Operators and expressions     | Mostly  |
| 4   | Classes, types, declarations  | Mostly  |
| 5   | Scoping, lookup, flattening   | Partial |
| 6   | Type relationships            | Minimal |
| 7   | Inheritance, redeclaration    | Full    |
| 8   | Equations                     | Mostly  |
| 9   | Connectors and connections    | Mostly  |
| 10  | Arrays                        | Mostly  |
| 11  | Statements and algorithms     | Partial |
| 12  | Functions                     | Mostly  |
| 13  | Packages                      | Mostly  |
| 14  | Overloaded operators          | Full    |
| 15  | Stream connectors             | Full    |
| 16  | Synchronous language elements | Full    |
| 17  | State machines                | Partial |
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
`assert` with its message, among the equations or among the actions of
a `when`, where it is made at the moment the event fires rather than at
every step; functions with several outputs
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

Arrived since this map was last revised on 18 September, each named by
the test that holds it: a function handed to another with its inputs
filled in, specialized one call deeper and inside a walked body
(`a_function_handed_on_is_specialized_one_call_deeper`,
`a_hand_over_inside_a_walked_body_is_specialized`); a walked body
that answers with a number and an array together
(`a_walked_answer_of_a_number_and_an_array_fills_both_targets`), with
an array as long as an input says
(`a_walked_answer_as_long_as_its_input_is_taken_element_by_element`),
with a record built by its constructor, with comprehensions, `linspace`
grids and lists written in place; least-squares fits, singular value
decompositions and the rank of a matrix answered here rather than by
LAPACK (`a_least_squares_fit_is_answered_here`,
`a_rank_is_worked_out_of_the_singular_values_of_a_written_matrix`);
tables read from comma-separated text and from compressed MATLAB
files (`a_comma_separated_table_is_read_past_its_header`,
`compressed_matlab_elements_are_read_one_after_another`); a `for`
equation inside an `if` branch only the run decides
(`a_loop_in_a_branch_the_run_chooses_is_written_out_into_it`) and a
loop among a model's statements left by a `break` the run decides
(`a_loop_left_on_a_condition_the_run_decides_is_written_out_with_a_flag`);
`der` of an expression linear in its moving names
(`der_of_a_linear_expression_is_the_same_of_the_derivatives`); index
reduction through `min`, `max`, `atan2` and an `abs` whose sign the
branch settles (`index_reduction_differentiates_min_and_max_as_the_branch_they_are`,
`index_reduction_differentiates_atan2_by_both_of_its_arguments`,
`a_branch_that_settles_a_sign_differentiates_the_abs_inside_it`); a
`when` over a Boolean vector named whole
(`a_when_over_a_vector_named_whole_fires_on_each_element`); `fixed`
handed down over arrays and as a parameter element
(`a_fixed_written_over_an_array_pins_each_element`,
`a_fixed_handed_down_as_a_parameter_element_pins_the_state`); and an
initial section one or two conditions over, solved where the
conditions agree and named where they do not
(`a_section_two_conditions_over_is_solved_if_they_agree_and_named_if_not`).

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
floating-point number at runtime. `quantity` is parsed and ignored;
`nominal` is read in one place only, as the first guess for an unknown
that has no `start` (`a_nominal_on_the_type_is_where_an_unknown_begins`),
and `stateSelect` is read by index reduction, below; numeric literals
never carry units, so `x = 5`
is accepted whatever `x` is declared in. `min` and `max` are the assertions the specification says
they are: a value settled before the run is refused by the checker,
and one the run produces stops it at the first output point past where
it left the bounds - the check is made where a row is written, so a
variable crossing its `max` at 0.5 on an interval of 0.1 is stopped at
0.6 and on 0.07 at 0.56. An
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

**Class restrictions** (ch. 4): what a class kind may hold and who may
reach into it. A declaration under `protected` is visible inside its
own class and in the classes extending it, and a class reaching across
a component into one - naming `a.hidden`, `a[1].hidden`, or modifying
it from the declaration with `Inner a(hidden = 5)` - is refused,
whether the component was declared or inherited and whether the member
was declared by the component's class or inherited by it. A `block` is
a model whose connectors all have a direction: one that holds a
connector nothing gave a direction is refused, the direction being read
from the declaration, from the short definition the connector came from
(`connector RealInput = input Real`) or from every signal it holds, and
an `expandable connector` exempt since a bus takes its directions from
the connections that fill it. A block also settles its own outputs, so
one declaring an output nothing in it or in its bases ever speaks of is
refused too - a `partial` block excepted, which is exactly the case
where the outputs are left for whoever finishes it.

A class declared under `protected` is kept back the same way: naming
`P.shut` from outside `P` is refused, whether the name is what a
component is declared to be, what a class extends, or what a call
names, while `P` itself and the classes inside it write it freely.

Balanced models (4.7) are enforced by their two exact consequences
rather than by the count itself. A block settles the outputs it
declares, above; and an `input` of a model or block is settled by
something - a value on its own declaration, a modifier or an equation
from the class holding it, or a connection - where a class reading its
own input settles nothing, since asking to be given one is what
declaring an input means. The standard library keeps both: no example
model of it has an input nothing gives a value to.

The count itself is not made per class, and what stands in its way was
measured rather than argued. Counting every instance of the
standard-library models that flatten started at 10 627 instances whose
equations and unknowns disagree and came down to 3 428 as each term
was put right - a record is not an instance of its own, a scalar
connector carries its direction on itself, a declaration value is an
equation of the class that wrote it. What is left is not the rule but
this compiler: a `when` clause carries no instance to be counted
against, a record-valued `input` yields no declaration equations, and
a class inherited down two paths of a diamond has its declarations
counted twice. Each of those is a flattening question rather than a
chapter 4 one, and until they are settled a count built on them would
refuse correct models.

**Scoping and flattening** (ch. 5) is the largest working part of this
compiler and the one this map has said least about. A name is looked up
where it was written and resolved under the name it was asked by: a
constant of a package reached from inside it, a function reached through
the component that holds it, a member of an array of records reached by
subscript, a declaration a base wrote reached through the `extends` that
took it. `inner`/`outer` pairs are matched up the tree of instances, and
a modifier written on an outer declaration reaches the inner one it
names. What is still partial is the shape of the pass rather than any
rule of it: instantiation, the settling of constants and the working out
of shapes happen in an order fixed by the code, so a length known to a
later pass cannot be asked for by an earlier one. Most refusals that
remain in this chapter are of that kind - the value is knowable and is
not knowable _there_ - and each is recorded in the analysis with the
place it is asked from.

**Type relationships** (ch. 6) are the thinnest layer here, and
deliberately so. Subtyping is approximated by the `extends` chain:
a class is accepted where its base is wanted, a `redeclare` is checked
against the `constrainedby` it was declared under, and nothing else is
compared. Structural subtyping - a class standing for another because
its members happen to match - is not implemented, and neither is the
distinction between a type and the class that names it. For the
standard library this has cost nothing measurable, which is why it
stays where it is; it would be felt first by code that redeclares
across unrelated hierarchies.

**Statements and algorithms** (ch. 11) are complete in their forms and
partial in one direction only. `if`, `for`, `while`, `when`, `break`,
`return`, `assert` and `terminate` are all worked, in functions and in
algorithm sections alike, and an algorithm section settles the
variables it assigns the way an equation section settles the ones it
names. Eighty-eight tests in `tests/flattening/algorithms.rs` hold
that behaviour. What is missing is the
other direction: an algorithm is a list of assignments carried out in
order, and where the specification allows a tool to treat a tick as
something to be solved rather than executed - an implicit method on a
clocked partition being the case that arises - this compiler refuses by
name instead. That refusal is the wall ch. 11 stands behind, and it is
named where it is met rather than left for the run to discover.

**Packages** (ch. 13) hold classes and constants and nothing else - a
parameter or a variable in one is refused, since a package has no
instance to own a value. The imports are all four forms (`import A.B;`,
`import C = A.B;`, `import A.B.*;`, `import A.{C, D};`), a wildcard
reaching a package's constants as well as its classes but outranked by
a component of the model with the same name; `extends` merges a base
package's constants into the derived one, later declarations winning;
and `encapsulated` is a wall a class named simply inside a package does
not reach past, so it must be imported or built in - the overloads under a
quoted operator symbol excepted, since they exist to serve their
record and still see it
(`an_encapsulated_package_does_not_see_out_of_itself`). The wall
stands for classes only: `walk_out` in
`crates/oxidelica-parser/src/flatten/lookup.rs` stops there, and a
constant of the enclosing package is still read by its simple name
inside an encapsulated one, in an equation and in a function body
alike, where 13.2.1.1 asks for a refusal. That is why the chapter is
not Full.

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
(`String(x)` of a variable), string variables in results, and most of
the string functions of ch. 12 - `length`, `substring`, `compare` and
`skipWhiteSpace` are answered (`the_string_bodies_written_outside_are_answered_here`).
An `assert` message built by joining keeps the parts that are text
and writes `?` for a number only the run knows
(`a_check_inside_a_branch_holds_only_while_the_branch_does`).

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
not exact between output points: what leaves depends on the output
grid, so a coarse interval gives a coarse answer (the probes at the
end of this map). Its arguments but
the two inflows are checked before the run.

Zero is the one number that fits a unit of any kind: a rate of change
may be nothing, and nothing has no dimension of its own. Everything
else is dimensionless until a declaration says otherwise.

`cardinality(c)` answers how many `connect` equations name a port,
which is settled while the connections are still in hand. The
specification deprecates it and says it will be removed in a coming
release, so a model is better off without it - but while it is defined
it is answered, including in the assertion it is nearly always asked
inside and in the `if` that decides whether an equation exists at all.
That last one is a question about the model as a whole asked while the
model is being built, so the first pass is kept for its connections
alone and everything is built again with the counts in hand - the same
two passes `Connections.isRoot` already needed.

Two forms the grammar gives a primary are refused as syntax errors: a
parenthesised expression followed by subscripts or by a member,
`(g(3))[1]` and `(f(3)).a`, and with them a subscript on an array
written in place, `{1, 2, 3}[2]`, which the grammar reaches only through
the parentheses. A call subscripted directly, `pair(2)[2]`, is read
(`a_call_may_be_subscripted_where_it_is_written`).

Chapter 3 is complete but for one wrong answer: `semiLinear` with
slopes that are names reads their difference at zero flow, where it
is zero, from a rewrite that supplies a stream's missing equation there
(the sixth entry of the probes at the end of this map).

**Arrays** (ch. 10) are nearly complete: literals, ranges, slicing with `end`,
comprehensions, elementwise operators and the matrix algebra
(`transpose`, `identity`, `diagonal`, `cross`, `outerProduct`,
`symmetric`, `skew`, `cat` and `[ , ; ]`), the reductions and the
constructors. A dimension may be a number, a type - `Real x[Boolean]`
has two elements indexed off `false`, `Real x[E]` one per enumeration
literal - or a `:` that reads its length from the value the component
is given.

A subscript the compiler cannot settle is read at the run instead, by
asking which place the index names: this is how the logic tables read
`NotTable[x]` off a signal. An index outside the array gives no number
at all rather than the nearest element - a `NaN` that is written to the
results, the run carrying on and ending as a success, rather than a
refusal naming the subscript. Two things are missing. Such a
subscript may be read but not assigned, so `v[k] = 9` with a `k` only
the run knows is refused, where the specification allows it. And an
index that is a `Boolean` _variable_ is refused for want of its
declared type at the point the subscript is worked out, though a
Boolean _expression_ - `x[a > b]` - is read correctly; the dimension
`Real x[Boolean]` itself is built either way.

**Functions** (ch. 12): a function may be handed to another as an
argument, `solve(function f(a = 2), 0, 1)`, which is how the standard
library asks for a root; the receiver is specialized into a copy that
calls the target directly rather than carrying a function value
(`a_function_may_be_handed_over_with_its_inputs_filled_in`). A body written
outside Modelica is answered where this compiler has an answer of its
own - the strings, the tables, the generators, the files, and of
LAPACK `dgesv`, `dgelsy`, `dgesvd`, `dgetrf` and `dgetri` - and
refused by name where it has none; [EXTERNAL.md](EXTERNAL.md) says
which and why. A function is inlined symbolically
wherever it can be, which is what lets the compiler differentiate
through one and fold it away where the arguments are known — and a
skipped tuple slot still costs the work of computing that output's
expression.

Where inlining cannot reach, the call is left standing and the run
walks the body for itself. Three things cannot be inlined: a function
that leads back to itself, directly or through others, which has no
bottom to unroll to; a `while` whose trip count the model decides
rather than the compiler; and a body that leaves by a `break` or a
`return` on a condition only the run settles, since which statements
run is what the leaving decides. So recursion and a data-dependent
loop both run, and `5!` written with an `if` statement comes out at
a hundred and twenty. Written as an `if` expression,
`r := if n <= 1 then 1 else n * f(n - 1)`, it is refused as calling
itself 64 deep: the walk works out both branches of an expression
before choosing, so the recursion has no bottom.

A walked body carries arrays, and records written out as their
members at the boundary. It may answer with more than one output, laid
end to end, where the targets are filled inside a `when`
(`a_walked_answer_of_a_number_and_an_array_fills_both_targets`), and
with an array as long as an input says
(`a_local_array_as_long_as_an_input_says_is_walked_to_its_value`).
What it may not do is answer with an array whose length the compiler
cannot see, or hold a `String` - a `when` has no meaning there either,
since there is no event inside a call. A walk that will not end is
stopped and named - 64 calls deep
(`a_body_nothing_could_inline_is_walked_by_the_run`), or ten million
rounds of one loop. And a walked body is opaque to everything the
symbolic layer does: it cannot be differentiated except through a
`derivative` annotation, and it is not folded where its arguments are
known.

A record is built by naming its fields or by giving them in order, and
a field left out takes the value its own declaration gives it; a
record that declares a `'constructor'` is built by that instead. A
handle to something outside Modelica - a table's data - is built the
same way, by name or in order. One shape is refused that should not
be: a variable record, not a parameter, whose fields have defaults and
which is bound by its constructor, `R r1 = R(b = 5)`, is refused as
unbalanced, the field's default and the binding both written as
equations. The same record bound as a parameter, or modified as
`R r1(b = 5)`, or with no defaults, comes out right.

A body reads its own locals before they leave it: `Integer m = size(x,
1)` is written in terms of an input, and left to be read later it
would carry the input's name out of the body, where the name means
nothing. One that comes to a number is stored as one, so an `if`
written on it is one arithmetic can decide; one that is an array keeps
its name, since an element is read off a name rather than off a list
written in its place.

A run of elements may be named along more than one axis - `oM[1:mBase,
1:mBase] := ...` - and is given that many values, the last subscript
moving fastest. A name bound whole and then written into element by
element is taken apart first.

An answer of more than one dimension is an array of arrays rather than
a flat list, since whoever was handed it reads `den2[i, 2]` and a
second subscript has nowhere to go in a flat one.

A tuple filled by a call - `(r, a, b, ku) = lowPass(cr, c0, c1,
f_cut)` - may stand at the top of a class or inside a branch the
compiler settles, and is read the same way in both.

A `when` written among a model's statements holds an algorithm like
any other - an `if`, a loop, a write to one element, a check - and
running it is what says which names it leaves changed and what each of
them is worth. That is what the standard library's Fourier transform
does: it counts ticks at an event and branches on the count.

Among a model's own statements a `while` is still unrolled where it
stands, so its trip count has to be settled there: only a call can be
left for the run to walk.

A function may say how to differentiate itself:
`annotation(derivative = f_der)` names a function taking what this one
takes and then one derivative for each input that has one - a `Real`,
or a library's alias of one, and neither an `Integer` nor a handle to
something outside - and the call is inlined for its value while keeping
that rule beside it. Differentiation reaches for
the rule instead of taking the body apart, which is what lets a body
the differentiator cannot read — one with `abs` in it — still carry a
model that needs its constraint differentiated or its Jacobian built.
Of the options the specification allows beside it, `zeroDerivative` is
read: `derivative(zeroDerivative = delta) = f_der` says the rule holds
wherever `delta` does not change with time, which is how the fluid
library writes its smoothing functions, and the inputs it names are
carried beside the rule. They are not yet checked to stand still: a
rule taken over an input that moves gives a wrong derivative (the
probes at the end of this map). That is what
lets a body with an `abs` in it carry a model needing its Jacobian
built. `noDerivative = aux` is read too: the rule is handed that input
as it stands and no derivative beside it, since the caller worked it
out from the other arguments - how the water library carries its
property record from one call into the next
(`an_auxiliary_the_caller_worked_out_is_handed_on_without_a_derivative`).
The remaining option, an order, changes what the named function
answers, so an annotation carrying one is read past and not kept
(`a_function_may_say_how_to_differentiate_itself`): the call stands
with no derivative rule of its own rather than a wrong one, and a
derivative asked of it is refused by name. Reading one wrong would give
a wrong derivative and nothing downstream could catch it.
`InlineAfterIndexReduction` is not read either - inlining here happens
before reduction or not at all, so honouring it would want a second
inlining pass that does not exist.
`annotation(inverse(x = f_inv(y)))` is read
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
record output field by field. A variable declared with a record value -
`Complex vs[m] = plug.pin.v` - says that value of its fields, since
there is no name in the flat model for a record itself. A parameter
may not become an equation, so its value is handed down as one
modifier per field instead; a field the record declares `final` is not
one a value may hand down, and a value that will not come apart is
left where it was written. An operator written for one record and
handed a whole array of them works on each in turn - `v1 - v2` of two
`Complex[3]` is three subtractions - which is the same vectorization
an ordinary function gets, and it reaches functions taking records
too: `abs(v)` of a `Complex[m]` is `m` calls. The record a value belongs to is still
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
ask for the held value by name. Where the inference cannot decide -
two clocks reaching one variable, a factor with nothing to settle it -
it refuses and names what it could not decide, rather than picking one
and carrying on.

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

Chapter 16 is complete in what it computes. One form is refused: the
constructors by argument name -
`Clock(condition = c, startInterval = 0.1)`,
`Clock(intervalCounter = 3, resolution = 10)`,
`Clock(c = k, solverMethod = "ExplicitEuler")` - since only `interval`
and `resolution` are known by name; every one of them works written by
position. Probed on 16 October with a count of ticks and an interval
worked by hand for each form, and those held.

**State machines** (ch. 17) are built on the clock: states are block
instances, `initialState` and `transition` declare the graph, and
`activeState`, `ticksInState` and `timeInState` answer from the
machine's own bookkeeping — `timeInState` counting periods, so a
machine on an event clock is refused it and keeps `ticksInState`. A
state's equations count only while it is in force, the rest hold what
they had, and an arrival puts a state's variables back to their start
values as `reset = true` asks.
A transition is judged on this tick's condition, and an immediate one
is taken on that tick, as 17.3.4 says; `immediate = false` keeps the
answer for a tick and is taken on what it kept. What 17.3.4 also says,
and this compiler does not do, is run the equations of the state that
is active on the tick it becomes so - which is the first of the four
defects below. `reset` belongs to
the arrow rather than to the state it arrives
at, so a state reached by two arrows starts over by the one that asked
and carries on by the one that did not. Two arrows out of one state
saying the same thing about which goes first are refused, as the
chapter requires.

The chapter is Partial, on Roman's word of 16 October, for four defects
found by probes on that day (models parked with the m439 notes, a test
for the first written and failing):

1. A state's equations do not run on the tick it becomes active, the
   first tick of the initial state included, and `ticksInState()`
   answers zero there where 17.1 counts one. 17.1's own example - `A1`
   entered at 10 ms, left on `ticksInState() >= 5` by a delayed arrow -
   is active here from 10 through 15 ms where the chapter says 10
   through 14.
2. The same defect seen from inside the state: a counter
   `n = previous(n) + 1` with `start = 0` reads 0 on the tick its state
   is entered, where 17.3.7's equivalent model, which writes
   `i_previous = if activeReset then 0 else previous(i)`, gives 1, and
   where the same counter outside any machine reads 1 on its first
   tick. Every state value is a tick behind what the chapter computes.
3. Two machines in one class are refused: a second `initialState` is
   "one initial state too many"
   (`crates/oxidelica-parser/src/parser/classes.rs`), so 17.3.7's own
   example, two parallel machines side by side in `State1`, does not
   read. Machines in separate components do run.
4. A state may not be empty: 17.1's own `block State end State;` is
   refused as "not a component with anything in it".

The tests that hold `reset`, `immediate` and the hierarchy were
written against this compiler's counts, so a repair of the first
defect has to read them again against 17.3.4 rather than trust them.
The nesting, `synchronize` and the merging of definitions below are
what works, judged on this compiler's own tick.

A model may hold several machines, each in a class of its own, and a
state of one machine may hold others:
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
each with the message its declaration wrote. `Evaluate = true` is a
proposal and not a demand - the chapter's word is `proposes`, beside
`Inline` and `smoothOrder` - so a parameter whose value the compiler
can settle takes the proposal and is evaluated, and one it cannot is
carried into the run with a note saying which binding defeated it,
which declines the offer. An offer declined is no broken law, where
refusing the model would be a wall built out of an annotation.
`fixed` may be any Boolean expression, as the language allows and as
the fluid valves use - `Av(fixed = CvData == CvTypes.Av)` - and the
expression is carried to where the enumeration is known rather than
being dropped or read as `false`. `experiment` says
where a run begins, how long it is and how often it writes - a
`StartTime` other than zero is the time the run starts at, which is
how the flux tubes sweep a coil from `-4` millimetres with `time`
standing for the position rather than for a clock - and `HideResult` leaves a
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
every one of the Modelica Standard Library's 2671 files parses, and of
its 1020 example models 953 flatten and 774 run. Of the 891 written to
be run - marked by an `experiment` annotation or the Example icon - 836
flatten and 731 run. Those are the numbers CI's library job printed
on 16 October (run 38837611219); [MSL.md](MSL.md) keeps them current.
What stands in the way of the rest is measured rather than guessed -
`oxidelica library check` ranks the reasons - and the list is in
MSL.md: the wall has moved from flattening to running, and the
families at it are structurally singular systems, algebraic loops,
unbalanced models and parameters with no value.

**Structure**: an `expandable connector` takes each member's
type from the other side of the connection that names it, so a member
connected only to another bus member has nowhere to get one, and
`each`-style array members of a bus are not supported. A member the bus
does not declare is also refused when what connects to it is a short
connector, `connector RealOutput = output Real` as the standard library
writes its signals: `connect(s.y, b.sig); connect(b.sig, k.u)` on an
empty bus is "unknown variable `b.sig`", on the bus itself, through a
joined sub-bus and across two components alike. Declared in the bus,
`expandable connector Bus Real sig; end Bus;`, or carried by a connector
with a field inside it, the same wiring runs; the library's own
`ControlBus` declares its members, which is why `BusUsage` runs. The
layer, by reading: `expand_buses` in
`crates/oxidelica-parser/src/flatten/connections.rs` mints a member by
instantiating the other side's class under the prefix `b.sig.`, which
makes `b.sig.y` of a connector with a field and nothing at all of a
short one, whose value is the connector itself. Every bus of the
standard library declares the members it carries, so no library model
meets this; a model of one's own that leaves the bus empty, as 9.1.3
allows, does. A library may be a directory tree, but
a file's place in it comes from its `within` clause rather than from
where the file sits, so a tree without `within` headers is read as
though it were flat.

A call whose body is written outside Modelica and which answers with
nothing is read and dropped: `Streams.print(...)` writes a line on a
terminal, there is none here, and no value goes missing. One that does
answer is answered where this compiler has an answer of its own, and
refused by name where it has none - never quietly given a number that
came from nowhere.

**Semantics**: `StateSelect` is the language's own enumeration, and
index reduction reads it where the weights it measures leave a choice
open: a tie goes against `never` first and `prefer` last
(`a_tie_between_positions_keeps_the_one_the_model_prefers`), and an
`always` stands aside from demotion wherever another candidate is
constrained (`a_state_the_model_says_is_always_one_is_not_demoted_while_another_can_be`).
It is not a demand the compiler enforces: where only the `always`
candidate is determined at all, it is demoted rather than leaving a
block nothing can solve, and the weights come before the attribute.
`der()` of a plain variable, or of an expression linear in its moving
names (`der(k*x + 3)`), though the equation holding one need not state
it: `der(x) + der(y) = 1` says
something about two derivatives at once and neither can be got out of
it, so each is solved for like any other unknown, which is what a DAE
is. A condition of a
run-time `if` equation must be readable from the parameters, the
states and plain `name = expr` definitions at the instant the mode is
settled — a condition that only an algebraic loop could produce falls
back to the `else`. A `when` may stand among the statements of
a model's algorithm section, but only at the top of one - inside it
may hold an algorithm of its own. `homotopy` takes the real problem and goes
straight at it, which the specification permits and which means no
continuation is run. The operators with declared argument names -
`homotopy`, `smooth`, `delay`, `semiLinear`, the clocked
conversions and `spatialDistribution` among them - take their arguments
by name as well as by position, and a name none of them declares, or one
given twice, is refused by that name. `Clock` knows two of its names,
`interval` and `resolution` (`crates/oxidelica-parser/src/flatten/names.rs`);
`intervalCounter`, `condition`, `startInterval`, `c` and
`solverMethod`, which 16.3 also names, are refused, so those clocks
must be written by position. `Connections.root`, `potentialRoot`, `branch`,
`isRoot` and `rooted` decide the roots of an overconstrained graph and
refuse one with no root or with two. `rooted(a)` answers by depth in
the spanning tree - whether `a` is the end of its branch nearer the
root - and not by whether `a` is itself a root, which are different
answers wherever a branch is longer than one edge. A connection that
closes a loop owes the record's `equalityConstraint(r1, r2)` and gets
it, so an overdetermined connector is balanced by the residue its own
function writes rather than by the equality a tree connection takes.
`delay(u, T)` keeps what `u` was at each output point and reads
between them in a straight line, so the shift is as exact as
`Interval` is fine and the step is never longer than the delay; `T`
must be known before the run - one that moves, `delay(u, T, Tmax)` with
`T = 0.1 + 0.1*time`, is refused, though as "unknown variable `T`"
rather than by the rule it breaks - and a continuation after a
re-compilation starts its memory over. Subtype
compatibility (ch. 6) is approximated by the `extends` chain for
`constrainedby`, though a connection set matches its connectors by
shape — same member names, same `flow` and `stream` prefixes — rather
than by class name.

## What this map says that no test holds

Each statement above was checked on 16 October against the 1141 tests
of the tree, by searching test bodies for the construct and reading
the test that came up. Most hold. These are the places where the map
says something and nothing in the test suite would turn red if it
stopped being true, each with its chapter. Each was then put to the
binary with a small model, and what came back is said beside it; the
models are kept outside the repository with the shift's notes. A probe
is not a test, so every one of these is still owed one.

Nine came back as wrong answers, with no refusal, and are first. The
last six were found by putting the chapters to the binary as well, and
moved chapters 3 and 13 to Mostly and chapter 17 to Partial:

1. Ch. 12: `zeroDerivative` is taken where the input it names moves.
   `f(x, d) = (1 + d)*x` with a body only the run can walk, a rule
   `f_der = (1 + d)*x_der` under `derivative(zeroDerivative = d)`, and
   `f(x, d) = 2 + time` with `d = 1 + time` makes `x` one and its rate
   zero; the run answers a rate of 0.5 falling to 0.33. With `d` a
   parameter, and with the annotation taken away, it answers correctly.
   The sentence above saying the rule "is taken only where they stand
   still" is therefore not what the compiler does, and is corrected.
2. Ch. 3: `spatialDistribution` is not exact between output points. A
   profile fed `time^2` and moved at unit speed gives `(t - 1)^2` at the
   far end; the run answers 1.12 at `t = 2` on an interval of 0.7 and
   1.00 on an interval of 0.1, so the answer depends on the output
   grid. Corrected above.
3. Ch. 8: a run-time `if` whose condition an algebraic loop settles
   runs its `else` until the first output point. With `x + y = 3`,
   `x - y = 1` and `if x > 1.5 then der(z) = 1 else der(z) = -1`, `z`
   falls for one output interval and then climbs, so `z(1)` is 0 on an
   interval of 0.5 where it should be 1. The sentence above says the
   condition "falls back to the `else`", which is true and is the
   defect: a silent branch is a wrong number, and a refusal is owed.
4. Ch. 17: a state entered on a tick does not run its equations on
   that tick, and `ticksInState()` answers zero there. 17.1's own
   example is active from 10 ms through 15 where it should stop at 14.
5. Ch. 13: the `encapsulated` wall stops classes and not constants: a
   constant of the enclosing package is read inside the wall, `y = 3`
   where a refusal is owed.
6. Ch. 3: `semiLinear(m, a, b)` with slopes that are names - a
   parameter or a variable, not a literal - reads `a - b` at zero flow
   where it is zero. The rewrite that supplies a stream's missing
   equation at zero flow (`semilinear_at_zero` in
   `crates/oxidelica-parser/src/flatten/mod.rs`) guards against exactly
   this with a test for plain names, and a parameter is a plain name
   too. A flow held at zero integrates the error: with `a = 2`, `b = 5`
   and `m = 0` until `t = 0.5`, an integral of `y` ends at -1.25 where
   0.25 is owed. Chapter 3 moved from Full to Mostly on this.
7. Ch. 8: a relation on a smooth signal that is not a state stops
   turning after its first switch or two, on the adaptive solvers,
   whenever the model has a continuous state. A Boolean
   `b = sin(20*time) > 0.5` beside `der(x) = 1` turns true at 0.026 and
   stays true to the end, where it should be false again at 0.131 and
   turn three more times; anything read through `b` is wrong for the
   rest of the run. A `when` on the same condition fires once where it
   rises four times, and `cos(20*time) < -0.5` fires never where it
   falls three times. Without the state, or on `rk4`, every crossing is
   found; a condition that is itself a state - a bouncing ball - is
   found too. The same relation written inline,
   `z = if sin(20*time) > 0.5 then 1 else 0`, turns every time but only
   at output points, with no event at the crossing. The step taken
   again short of a crossing (`0bf1b70`, 14 October) took the Boolean
   from three turns of seven to one; the commit before it already lost
   four, and the `cos` case is older still. The indicators are compared
   at the two ends of a step only, and a smooth state lets the step grow
   past half the signal's period, so two crossings inside one step
   cancel; a state that forces short steps finds all seven. It reaches
   the standard library's own blocks: `Sources.Sine` into
   `Logical.GreaterThreshold(threshold = 0.5)` through `BooleanToReal`
   into `Continuous.Integrator` ends at 0.974 on `dopri45` and 0.136 on
   `bdf` where 0.346 is owed, and `rk4` gives 0.340; through
   `Logical.Hysteresis(uLow = -0.5, uHigh = 0.5)` the same integral is
   0.974 on `dopri45` where 0.503 is owed. A `BooleanPulse`, whose
   switches are time events known in advance, integrates exactly. A
   run-time `if`
   equation on the same relation - the ideal switch of chapter 8 -
   sees every turn but takes each at the next output point, 0.140 for a
   crossing at 0.131 on an interval of 0.01. Chapter 8
   stays at Mostly, where it already stood.
8. Ch. 3: a parameter or a constant bound to `time` -
   `parameter Real m = 2*time`, `constant Real m = 2*time`,
   `parameter Real m = sin(time)` - is accepted and holds its value at
   the start time for
   the whole run, where 3.8 says a binding may be no more variable than
   the name it binds and a refusal is owed. One bound to a plain
   variable is refused, naming it. Chapter 3 is already at Mostly.
9. Ch. 8: a parameter left free by `fixed = false` and settled by an
   initial equation that reads a state is solved and then not used
   except as an alias. With `initial equation p = 2 * x`, a variable
   `q = p` reads 2, while `y = p * x` reads 0 and `der(x) = -p * x`
   leaves `x` at 1 where `exp(-2)` is owed. Settled by `p = 2`, with no
   state in the equation, it is right. The library's one running example
   with such a parameter, `HeatTransfer.Examples.TwoMasses`, settles
   `T_final_K` at 323.15 as it should and reads it nowhere, which is why
   no floor met this.

The rest came back as the map says:

1. Ch. 12: ten million rounds of one loop. The ceiling stands in
   `crates/oxidelica-sim/src/walk.rs` (`MAX_ROUNDS`) and no test reaches
   it; the 64-call ceiling beside it is tested. Probed, it fires: a
   `while` that never ends is refused at `t = 0` with "took more than
   10000000 rounds", in two and a half seconds and 190 MB, which is
   cheap enough to be a test.
2. Ch. 8: `delay` starting its memory over after a re-compilation. No
   test re-compiles a model holding a `delay`. Not probed.
3. Ch. 9: a connection set matches connectors by shape. Two connector
   classes with the same members connect and run; two with different
   members are refused, naming both classes.
4. Ch. 9: a bus member connected only to another bus member is refused
   with "a bus member takes its type from the other side", and an array
   handed to a bus member is refused as a connection between two and one
   connectors.
5. Ch. 13: a library tree without `within` headers is read as though
   it were flat: `L.Sub.K` is not found, a bare `K` is.
6. Ch. 14: an operator whose result is a different record than its
   operands is said to be misread. `A * A` answering a `B` came out
   right in the one shape probed, so the sentence is not confirmed and
   not refuted: a test is wanted that either shows the misreading or
   lets the sentence go.
7. Ch. 18: `Protection` and `uses` are read without complaint. Whether
   they are carried is not visible from a run; `connectorSizing` and
   `conversion` were not probed. `Dialog` and `Evaluate` are tested
   (`crates/oxidelica-parser/tests/flattening/names.rs`).
8. Ch. 4: the count of 10 627 instances down to 3 428 is a measurement
   with no file beside it and no script that prints it, so it cannot be
   taken again as it stands.
9. Ch. 6: structural subtyping is said to have "cost nothing
   measurable" on the standard library. Nothing measures it: no
   refusal in the census is tagged as a type relation, so the claim
   is unfalsifiable as written.

Two sentences were found false against tests and are corrected above
rather than listed: functions handed as arguments (said to be absent,
tested since 31 August) and a walked body answering with several
outputs (said to be refused, tested since 26 September). Six more were
narrower than the tree: `nominal`, `stateSelect`, `noDerivative`, `der`
of an expression, the joined `assert` message and the string
functions.

One more defect turned up on the way, written here rather than fixed: a
tuple `(p, q) = f(u)` among the continuous equations, where `f` holds
a `while` only the run can end, is refused with `the trip count of a
loop is not settled here`, though the same call inside a `when`, or
written as `p = f(u)`, runs. The refusal names the wrong wall.

What this pass does not see is the other half. Tests added in a month
show what arrived; what narrowed or broke shows in the census and in
the families of refusals, which this pass did not read.
