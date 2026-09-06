// A medium's `constant h_default` whose value is a call whose body
// iterates deeply, read by a parameter through a replaceable package -
// the exact shape of the ReferenceAir media that head the corpus
// queue, and this is the case where all of a shift's C-fluid fixes are
// needed together.
//
// `h_default = specificEnthalpy_pT(p_default, T_default)`, whose body
// is `enthalpy(setState(p, T))`, and the medium redeclares `setState`
// to build a state whose enthalpy divides by a density the medium
// computes with a Newton `while` over a polynomial dozens of terms
// deep. The constant road cannot make a number of it - the body is
// past the depth it follows - so `class_constant_at` fails, and the
// name reached the parameter bare and was refused as `cannot evaluate
// parameters [...]: nothing gives a value`.
//
// The fix hands the parameter the constant's binding rather than its
// bare name (constants.rs), with the sibling constants folded into the
// call's arguments, so the parameter's own deeper walk - which folds a
// deep numeric field while a parameter settles - can work the call out
// to a number. Read without the fix this model is refused; with it the
// enthalpy folds. It is the guard the corpus census (62 media
// `h_default` barriers cleared) could not be.
package P
  partial package Base
    constant Real p_default = 100000;
    constant Real T_default = 293;
    replaceable record State Real h; Real d; end State;
    replaceable partial function setState
      input Real p; input Real T; output State s; end setState;
    replaceable partial function enthalpy
      input State s; output Real h; end enthalpy;
    function specificEnthalpy_pT
      input Real p; input Real T; output Real h;
    algorithm h := enthalpy(setState(p, T)); end specificEnthalpy_pT;
    constant Real h_default = specificEnthalpy_pT(p_default, T_default);
  end Base;
  package Air
    extends Base;
    record HD Real f; Real pd; end HD;
    function helm
      input Real d; input Real T; output HD r;
    protected
      final constant Real[19] N = {0.11,0.71,0.61,0.07,0.08,0.13,0.01,0.04,0.03,0.0001,0.10,0.17,0.04,0.01,0.14,0.03,0.0002,0.014,0.009};
    algorithm
      r.f := 0;
      for k in 1:19 loop r.f := r.f + N[k]*d^k*T^(0.001*k); end for;
      r.pd := 287*T;
    end helm;
    function density_pT
      input Real p; input Real T; output Real d;
    protected Integer i=0; Boolean found=false; Real dp; HD f;
    algorithm
      d := p/(287*T);
      while ((i<100) and not found) loop
        f := helm(d, T);
        dp := d*287*T - p + 0.0001*f.f;
        if abs(dp) <= 1e-6 then found := true; end if;
        d := d - dp/f.pd; i := i+1;
      end while;
    end density_pT;
    redeclare record extends State Real d; end State;
    redeclare function extends setState
    algorithm s := State(h = 1000*T + p/density_pT(p, T), d = density_pT(p, T)); end setState;
    redeclare function extends enthalpy
    algorithm h := s.h; end enthalpy;
  end Air;
  model Use
    replaceable package Medium = Base;
    parameter Real h = Medium.h_default;
    Real z(start = h, fixed = true);
  equation der(z) = 0;
    annotation(experiment(StopTime = 1)); end Use;
  model M Use u(redeclare package Medium = Air); end M;
end P;
