// A field of a record-valued constant, built by a modifier list, read
// by a parameter.
//
// `constant FundamentalConstants Constants(R_s = 287.117, ...)` is how
// the ReferenceAir medium writes its gas constants: a record component
// with `constant` variability whose fields are set by modifiers, not
// by an equation. Reading `Constants.R_s` asks a class named
// `Constants` - which is not one, it is a component - so the ordinary
// constant road bailed and the name reached the flat model bare.
//
// This was the true wall in front of C-fluid. The air density
// iteration `dofpT` starts from `d := p/(R_s*T)`, and with `R_s` a
// bare name the whole `while` stayed symbolic, so `h_default` never
// came to a number and every media parameter reading it was refused.
// The while loop and the redeclared record - the two earlier suspects
// - both fold once the gas constant does.
//
// The fix takes the head apart into the package that holds the record
// and the component's own name, finds it there, and reads the field
// off its modifiers; a field the modifiers leave out is read from the
// record's own declaration default. Both are exercised here: `R_s`
// comes from the modifier list, `MM` from the record's default.
package P
  record FundamentalConstants
    Real R_s;
    Real MM = 0.0289586;
  end FundamentalConstants;
  package Basic
    constant FundamentalConstants Constants(final R_s = 287.117);
  end Basic;
  model Use
    parameter Real gas = Basic.Constants.R_s;
    parameter Real molar = Basic.Constants.MM;
    Real z(start = gas + 1000*molar, fixed = true);
  equation
    der(z) = 0;
    annotation(experiment(StopTime = 1));
  end Use;
end P;
