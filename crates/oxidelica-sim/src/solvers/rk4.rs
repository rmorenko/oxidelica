//! Classical Runge-Kutta on a fixed grid.

use crate::*;

impl CompiledModel {
    /// Classic fixed-step RK4 integration over `[0, stop_time]`.
    pub fn simulate_rk4(&self) -> Result<SimResult, SimError> {
        let n = self.states.len();
        let steps = (self.stop_time / self.step).ceil() as usize;
        let mut y = self.initial.clone();
        let mut alg_guess = self.algebraic_start.clone();
        let mut values = self.values_template.clone();
        let (mut k1, mut k2, mut k3, mut k4) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut scratch = vec![0.0; n];

        let mut columns = vec!["time".to_string()];
        columns.extend(self.states.iter().cloned());
        columns.extend(self.output_algebraics.iter().map(|(name, _)| name.clone()));
        columns.extend(self.discretes.iter().cloned());
        let mut rows = Vec::with_capacity(steps + 1);

        let mut record = |t: f64,
                          y: &[f64],
                          values: &mut [f64],
                          k: &mut Vec<f64>,
                          this: &CompiledModel,
                          alg_guess: &mut [f64]|
         -> Result<(), SimError> {
            this.eval_point(t, y, values, k, alg_guess)?;
            this.remember_delays(t, values);
            this.check_asserts(t, values)?;
            let mut row = Vec::with_capacity(1 + this.states.len() + this.algebraics.len());
            row.push(t);
            row.extend_from_slice(y);
            for &(_, slot) in &this.output_algebraics {
                row.push(values[slot]);
            }
            for &slot in &this.discrete_slots {
                row.push(values[slot]);
            }
            rows.push(row);
            Ok(())
        };

        // A model with time events needs a solver that can step onto
        // them; the fixed grid of RK4 cannot.
        if !self.samples.is_empty() {
            return err(
                "sample() needs a solver that steps onto the event: use dopri45 or bdf".to_string(),
            );
        }
        let mut state = self.event_state();
        values[self.initial_slot] = 1.0;
        record(0.0, &y, &mut values, &mut k1, self, &mut alg_guess)?;
        let mut terminated = self
            .handle_event(0.0, &mut y, &mut values, &mut alg_guess, &mut state)?
            .terminated;

        for i in 0..steps {
            if terminated.is_some() {
                break;
            }
            let mut t = i as f64 * self.step;
            let end = (t + self.step).min(self.stop_time);
            // A relation on `time` turns at an instant known before the
            // run, and the grid need not land on it: the step is split
            // there so that no stage reads the far side of the switch
            // while the step is still on the near one.
            while t < end - 1e-12 {
                let h = match state.next_time_event() {
                    Some(next) if next > t + 1e-12 && next < end - 1e-12 => next - t,
                    _ => end - t,
                };
                self.eval_point(t, &y, &mut values, &mut k1, &mut alg_guess)?;
                for j in 0..n {
                    scratch[j] = y[j] + 0.5 * h * k1[j];
                }
                self.eval_point(t + 0.5 * h, &scratch, &mut values, &mut k2, &mut alg_guess)?;
                for j in 0..n {
                    scratch[j] = y[j] + 0.5 * h * k2[j];
                }
                self.eval_point(t + 0.5 * h, &scratch, &mut values, &mut k3, &mut alg_guess)?;
                for j in 0..n {
                    scratch[j] = y[j] + h * k3[j];
                }
                self.eval_point(t + h, &scratch, &mut values, &mut k4, &mut alg_guess)?;
                for j in 0..n {
                    y[j] += h / 6.0 * (k1[j] + 2.0 * k2[j] + 2.0 * k3[j] + k4[j]);
                }
                t += h;
                if t < end - 1e-12 {
                    // Inside the grid step, on a scheduled instant.
                    self.eval_point(t, &y, &mut values, &mut k1, &mut alg_guess)?;
                    self.raise_time_events(t, &mut state, &mut values);
                    terminated = self
                        .handle_event(t, &mut y, &mut values, &mut alg_guess, &mut state)?
                        .terminated;
                    if terminated.is_some() {
                        break;
                    }
                }
            }
            if terminated.is_some() {
                record(t, &y, &mut values, &mut k1, self, &mut alg_guess)?;
                break;
            }
            let t = end;
            record(t, &y, &mut values, &mut k1, self, &mut alg_guess)?;
            self.raise_time_events(t, &mut state, &mut values);
            terminated = self
                .handle_event(t, &mut y, &mut values, &mut alg_guess, &mut state)?
                .terminated;
        }

        // See `finish_segment`: reaching the stop time is an event of
        // its own, and this solver has to raise it as the others do.
        if terminated.is_none() {
            values[self.terminal_slot] = 1.0;
            let stop = self.stop_time;
            let outcome =
                self.handle_event(stop, &mut y, &mut values, &mut alg_guess, &mut state)?;
            if outcome.changed {
                record(stop, &y, &mut values, &mut k1, self, &mut alg_guess)?;
            }
            terminated = outcome.terminated;
        }

        Ok(SimResult {
            columns,
            rows,
            parameters: self.parameters.clone(),
            terminated,
            method: SolverMethod::Rk4,
            reselections: 0,
        })
    }
}
