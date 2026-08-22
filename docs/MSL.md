# The Modelica Standard Library

The Modelica Standard Library (MSL) is what a Modelica tool is measured
against: 2671 files of models nobody wrote for this compiler. What
follows is how to get it, what it does here, and what it does not.

Russian version of this file: [MSL.ru.md](MSL.ru.md).

## Getting it

```bash
oxidelica library add modelica
```

That fetches [the library](https://github.com/modelica/ModelicaStandardLibrary)
at its 4.1.0 release into `~/.local/share/oxidelica/libraries/Modelica`,
which is one of the places the search already looks — so a model may
name `Modelica.Blocks.Sources.Sine` straight away, with nothing else set.
`XDG_DATA_HOME` moves that directory where it is set.

The fetching is `git clone --depth 1 --branch v4.1.0`, run as a command
rather than spoken over HTTP from here: a library is a git repository,
git checks what it downloads against the tag it was asked for, and this
compiler stays free of a network stack of its own.

Other libraries are fetched the same way, by URL:

```bash
oxidelica library add https://github.com/example/Library.git --version v2.0 --as Example
```

`oxidelica library list` says which libraries are in view and how many
files each holds. A library already on disk needs no fetching:
`MODELICAPATH` names one, or several.

## What it does

`oxidelica library check <directory>` reads every file of a library and
every example model in it, and says how far each got. At the time of
writing, against MSL 4.1.0:

```text
files: 2646 read, 25 not read
classes: 6539; example models: 733, of which 381 flatten and 37 run
```

Three numbers, and they mean different things. **Read** is the parser:
the file was understood as Modelica. **Flatten** is the front end: the
model was instantiated, its connections resolved, its arrays and
functions expanded, and what came out is a flat system of equations.
**Run** is the rest: that system came out as something solvable and
took ten steps without complaint.

The gap between the last two is the point of counting them apart. For
a long time only the middle number was measured, and it flattered the
compiler: a flat system nothing can solve is not an answer, and two
faults found while this was being written - a class inherited by two
paths of a diamond duplicating every variable, an array of records
given its value the wrong way round - had produced wrong flat models
for months without a single refusal to show for it. Flattening says
the front end had no objection. Running is what says the objection
would have been worth having.

Ten steps rather than a whole simulation, and never past where the
model itself stops. What goes wrong at this stage goes wrong at the
start - an unbalanced system, a parameter nothing settles, a name that
means nothing - and a full run would say little more for minutes
apiece.

Both numbers move as files start to parse, and not always upwards. A
file nobody could read holds classes nobody could name, and a call to a
name nothing defines is left standing rather than refused — so a model
that reached into an unread file could flatten while quietly carrying a
call to nothing. When the file starts parsing, the call finds its
declaration, the declaration says `external "C"`, and the model is
refused by name. That is the honest number arriving rather than a step
back: 26 models moved that way when `Modelica/Blocks/Tables.mo` began
to parse.

A worked example — an RC circuit built entirely from MSL components:

```modelica
model MslRc "An RC circuit built from Modelica.Electrical.Analog"
  Modelica.Electrical.Analog.Basic.Resistor r(R = 100);
  Modelica.Electrical.Analog.Basic.Capacitor c(C = 1e-4, v(start = 0, fixed = true));
  Modelica.Electrical.Analog.Sources.ConstantVoltage source(V = 5);
  Modelica.Electrical.Analog.Basic.Ground ground;
equation
  connect(source.p, r.p);
  connect(r.n, c.p);
  connect(c.n, source.n);
  connect(source.n, ground.p);
  annotation(experiment(StopTime = 0.05, Interval = 0.001, Tolerance = 1e-10));
end MslRc;
```

`RC = 100 × 1e-4 = 0.01 s`, so at `t = 0.05` the closed form is
`5(1 − e⁻⁵) = 4.9663103`. The run answers `4.966310`.

Another, where the block asks C for its answer and this compiler
writes the answer out instead:

```modelica
model TableRamp "A table of the standard library, read where the model wrote it"
  Modelica.Blocks.Sources.CombiTimeTable t(table = [0, 0; 1, 2; 2, 6]);
  Real y;
equation
  y = t.y[1];
  annotation(experiment(StopTime = 1.5, Interval = 0.5, Tolerance = 1e-10));
end TableRamp;
```

`[0, 0; 1, 2; 2, 6]` is two straight lines, a slope of two up to
`t = 1` and a slope of four past it, so at `t = 1.5` the closed form is
`2 + 0.5 × 4 = 4`. The run answers `4.000000`, and puts an event at the
corner. `Modelica.Blocks.Tables.CombiTable1Ds` reads the same way with
`t.u` for the abscissa. What is behind both is in
[EXTERNAL.md](EXTERNAL.md).

These need the library; the models in `examples/` do not, which is why
they live here.

## What it does not

The 25 files that will not parse, and the example models that will not
flatten, are not a long tail of small things. They are a handful of
features, each used widely:

- **External C bodies** — the time tables above all, then the random
  generators, the LAPACK bindings, the file reading. The strings and
  the one-dimensional tables are answered here now; the rest parse and
  are refused where they are called, naming what was asked for.
  [EXTERNAL.md](EXTERNAL.md) says how that is to change.
- **A local assigned in one branch only** — 41 models, the water
  properties of the media library among them.
- **A record component's declaration value** — `Complex v[m] = ...`
  says nothing to the model, where an equation of the same shape
  does.
- **The multibody library** — on a chain of lengths and shapes its
  visual parts are drawn with.
- **`Clock` with named arguments** — the clocked library writes
  `Clock(interval = 0.1)`.

Two groups of models, 73 between them, are refused for none of those
reasons: working them out does not finish. What was measured about that,
and what two measurements said wrongly, is in
[EXPANSION.md](EXPANSION.md).

The list is measured rather than guessed: `library check` ranks the
reasons by how often each came up, and that ranking is what the work
follows. `library check --list` names the models that flatten, which is
what tells a step forward from a step sideways.

## Compatibility as a rule

A library is a pile a model uses a corner of. A file of it that will
not parse is set aside rather than made everyone's problem: the model
beside it still loads, and one that needed something from the file it
could not read fails by name further in. That is what makes a number
like "381 of 733" mean anything — without it, one unparsed file would
make the whole library unusable and the number would be zero.
