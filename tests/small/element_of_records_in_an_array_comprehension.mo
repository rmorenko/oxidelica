// `{real(v[k]*conj(i[k])) for k in 1:m}` - the way the quasi-static
// libraries write active power. The subscript is the loop variable, so
// by the time the operator is chosen the element is a flat name,
// `v[1]`, and the record it is one of is known only under `v`.
// Refused as `unknown variable `v[1]` in equation`.
model ElementOfRecordsInAnArrayComprehension
  package P
    operator record C
      Real re;
      Real im;
      encapsulated operator '*'
        function mul
          import P.C;
          input C c1;
          input C c2;
          output C c3;
        algorithm
          c3.re := c1.re*c2.re - c1.im*c2.im;
          c3.im := c1.re*c2.im + c1.im*c2.re;
        end mul;
      end '*';
    end C;
    function re
      input C c;
      output Real y;
    algorithm
      y := c.re;
    end re;
  end P;
  parameter Integer m = 2;
  P.C v[m];
  P.C i[m];
  Real power[m] = {P.re(v[k]*i[k]) for k in 1:m};
  Real out;
equation
  for k in 1:m loop
    v[k].re = k;
    v[k].im = 0;
    i[k].re = 2*k;
    i[k].im = 0;
  end for;
  out = power[1]*time;
  annotation(experiment(StopTime = 1, Interval = 1));
end ElementOfRecordsInAnArrayComprehension;
