// Squeezed from Modelica.Electrical.QuasiStatic.Machines.Examples.
// TransformerTestbench to a source, a Yd01 transformer and a load,
// and standing for a reduction that spent states on a row the system
// already held.
//
// Every quasi-static component writes `omega = der(reference.gamma)`,
// and the written aliases tie all the angles of a network together.
// With a delta on the secondary the matching stumbles four reductions
// running on one alias, `transformer.l1sigma.plug_n.reference.gamma =
// transformer.l1sigma.plugToPins_n.plug_n.reference.gamma`, and each
// time it differentiates to `der(transformer.l2sigma.plug_p.reference.
// gamma) - transformer.l1sigma.inductor[1].omega` - a row the system
// already derives from an earlier reduction's `l2sigma.omega -
// l1sigma.inductor[1].omega` and `l2sigma.omega = der(l2sigma.plug_p.
// reference.gamma)`. Each copy demoted a state, and the last reduction
// was left with `314.159 - 314.159 = 0` and nothing to take:
//
//   structurally singular model: equation Bin(Sub, Ref("der(transformer.
//   l2sigma.plug_p.reference.gamma)"), Ref("transformer.l1sigma.
//   inductor[1].omega")) = Number(0.0) constrains no state
//
// With such a member refused and the next one of the singular subset
// tried, the model runs: 314.159 rad/s on both sides, and the source's
// 25284.8 W against the load's 24952.1 W leaves 332.7 W, the copper of
// the two windings. With the rule taken back (`OXIDELICA_IMPLIED_ROW=0`)
// it refuses as above, and `scripts/victim_witness.sh` asks both of
// one binary. The rule was an opt-in while the corpus under it went
// past its memory ceiling (docs/ANALYSIS.md, m402) and is on by default
// since the ladder that cost the memory was stopped (m403).
model a_delta_secondary_does_not_spend_its_angles_twice
  import QS = Modelica.Electrical.QuasiStatic;
  QS.Polyphase.Sources.VoltageSource source(f=50, V=fill(100, 3), m=3);
  QS.Polyphase.Basic.Star starS(m=3);
  QS.SinglePhase.Basic.Ground groundS;
  QS.Machines.BasicMachines.Transformers.Yd.Yd01 transformer(n=1, R1=0.01,
    L1sigma=0.001, R2=0.01, L2sigma=0.001, T1Ref=293.15, T2Ref=293.15,
    alpha20_1=0, alpha20_2=0, T1Operational=293.15, T2Operational=293.15);
  QS.Polyphase.Basic.Resistor r(m=3, R_ref=fill(1, 3));
  QS.Polyphase.Basic.Star starL(m=3);
  QS.SinglePhase.Basic.Ground groundL;
equation
  connect(source.plug_n, starS.plug_p);
  connect(starS.pin_n, groundS.pin);
  connect(source.plug_p, transformer.plug1);
  connect(transformer.plug2, r.plug_p);
  connect(r.plug_n, starL.plug_p);
  connect(starL.pin_n, groundL.pin);
  annotation(experiment(StopTime=0.02));
end a_delta_secondary_does_not_spend_its_angles_twice;
