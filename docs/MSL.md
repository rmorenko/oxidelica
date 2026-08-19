# The Modelica Standard Library

The Modelica Standard Library (MSL) is what a Modelica tool is measured
against: 2674 files of models nobody wrote for this compiler. What
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
files: 2636 read, 38 not read
classes: 5717; example models: 670, of which 268 flatten
```

Two numbers, and they mean different things. **Read** is the parser:
the file was understood as Modelica. **Flatten** is the whole front end:
the model was instantiated, its connections resolved, its arrays and
functions expanded, and what came out is a flat system of equations. A
model that flattens is a model this compiler can go on to simulate.

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

## What it does not

The 38 files that will not parse, and the example models that will not
flatten, are not a long tail of small things. They are a handful of
features, each used widely:

- **The multibody library** — 27 models, on a chain of lengths and
  shapes its visual parts are drawn with.
- **`Clock` with named arguments** — 16 models of the clocked library
  write `Clock(interval = 0.1)`.
- **A member read across an array of components in a `connect`** — 20
  models of the power-converter library.
- **External C bodies** — the tables, the LAPACK bindings, the file
  reading. These parse and are refused where they are called, which is
  the honest answer: there is no C here to run.

The list is measured rather than guessed: `library check` ranks the
reasons by how often each came up, and that ranking is what the work
follows. `library check --list` names the models that flatten, which is
what tells a step forward from a step sideways.

## Compatibility as a rule

A library is a pile a model uses a corner of. A file of it that will
not parse is set aside rather than made everyone's problem: the model
beside it still loads, and one that needed something from the file it
could not read fails by name further in. That is what makes a number
like "268 of 670" mean anything — without it, one unparsed file would
make the whole library unusable and the number would be zero.
