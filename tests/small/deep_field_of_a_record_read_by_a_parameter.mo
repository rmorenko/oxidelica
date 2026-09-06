// A parameter whose value is a field of a record a function builds,
// where that field is an arithmetic expression deeper than the compiler
// follows - a media property is a polynomial dozens of terms deep over
// constants, and the air density iteration `dofpT` reads exactly such a
// field. Carried whole the tree runs past MAX_DEPTH and the record
// call is left standing, so the field read becomes `build(x)[1]`, an
// index the parameter settler cannot evaluate.
//
// The value is finite and known: a number the compiler could compute.
// The fix, at the depth guard and only while a parameter is being
// settled - where a number is the whole of what is wanted and no run
// stands behind the value to walk what would otherwise stand - folds
// the deep expression to its number rather than refusing it. Gated on
// the parameter mark because an equation's deep expression is meant to
// stand for the run: an ungated fold at the same boundary cost six
// multibody and table models that relied on that.
package P
  record R
    Real f;
  end R;
  function build
    input Real x;
    output R r;
  protected
    Real t;
  algorithm
    t := x;
    for k in 1:40 loop
      t := t*1.01 + 1;
    end for;
    r.f := t;
  end build;
  function readf
    input Real x;
    output Real y;
  protected
    R r;
  algorithm
    r := build(x);
    y := r.f;
  end readf;
  model Use
    parameter Real v = readf(2.0);
    Real z(start = v, fixed = true);
  equation
    der(z) = 0;
    annotation(experiment(StopTime = 1));
  end Use;
end P;
