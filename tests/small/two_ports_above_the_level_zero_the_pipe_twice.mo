// Two vessels whose outlets sit above the level, joined by a pipe that
// stores nothing. Cut down from TanksWithOverflow: lowerTank.ports[2]
// and upperTank.ports[3] at height 6 with the level at 2, both in the
// OpenTank branch that writes ports[i].m_flow = 0 (Vessels.mo:381).
// With both ends zero the pipe's two balances hold the same flow twice,
// and the run refused with "equation Ref("m2") = Number(0.0)
// constrains no state" - no closure of section 9.2 is involved.
model two_ports_above_the_level_zero_the_pipe_twice
  "two vessels whose ports sit above the level, joined by a massless pipe"
  parameter Real h = 6;
  Real L1(start = 2, fixed = true);
  Real L2(start = 2, fixed = true);
  Real m1, m2, p1, p2, s1, s2, mp;
equation
  der(L1) = -m1;
  der(L2) = -m2;
  if L1 >= h then
    p1 = L1; s1 = L1 - h;
  else
    m1 = 0; s1 = p1 - L1;
  end if;
  if L2 >= h then
    p2 = L2; s2 = L2 - h;
  else
    m2 = 0; s2 = p2 - L2;
  end if;
  m1 + mp = 0;
  mp + m2 = 0;
  p1 - p2 = mp;
end two_ports_above_the_level_zero_the_pipe_twice;
