//! What a run carries when the model has to be built again
//! mid-flight, and the memory a `delay` reads from.

use crate::*;

/// What a remembered trace was at a moment, straight between the two
/// points either side of it.
pub(crate) fn look_back(trace: &[(f64, f64)], at: f64) -> f64 {
    let Some((first_time, first_value)) = trace.first().copied() else {
        return 0.0;
    };
    if at <= first_time {
        return first_value;
    }
    let mut previous = (first_time, first_value);
    for &(time, value) in trace.iter().skip(1) {
        if time >= at {
            let span = time - previous.0;
            if span <= 0.0 {
                return value;
            }
            let across = (at - previous.0) / span;
            return previous.1 + across * (value - previous.1);
        }
        previous = (time, value);
    }
    previous.1
}

/// Glue a continuation onto the rows already produced.
///
/// The variable set is identical between segments but the column order
/// is not: the states of each selection come first in its own rows, so
/// a continuation's rows are reordered into the first segment's layout
/// by name before they are appended.
pub(crate) fn append_segment(mut merged: SimResult, segment: SimResult) -> SimResult {
    let mapping: Vec<usize> = merged
        .columns
        .iter()
        .map(|name| {
            segment
                .columns
                .iter()
                .position(|other| other == name)
                .expect("both segments name the same variables")
        })
        .collect();
    for row in &segment.rows {
        merged
            .rows
            .push(mapping.iter().map(|&from| row[from]).collect());
    }
    merged.terminated = segment.terminated;
    merged.method = segment.method;
    merged
}

impl CompiledModel {
    /// Whether the state selection is still the right one at this point.
    ///
    /// A constraint must determine its demoted victim, so the victim's
    /// sensitivity has to stay comparable to the best alternative. Once
    /// it falls below a fraction of one, the pivot that made the choice
    /// would choose differently now - and it is asked to, while the
    /// algebraic layer is still far from singular. The margin is why
    /// the switch happens in clean territory rather than at the wall.
    pub(crate) fn selection_sound(&self, values: &[f64], time: f64) -> bool {
        self.selection_monitor.iter().all(|(own, alternatives)| {
            let own = own.run(values, time).abs();
            let best = alternatives
                .iter()
                .map(|code| code.run(values, time).abs())
                .fold(0.0f64, f64::max);
            own >= 0.15 * best
        })
    }

    /// Put each delayed value in its slot, read from what the run has
    /// remembered. Before the delay has elapsed the answer is the
    /// value the run started from, as the specification asks.
    pub(crate) fn fill_delays(&self, t: f64, values: &mut [f64]) {
        if self.delays.is_empty() {
            return;
        }
        let history = self.history.borrow();
        for (delay, trace) in self.delays.iter().zip(history.iter()) {
            values[delay.slot] = look_back(trace, t - delay.seconds);
        }
    }

    /// Remember what each delayed expression is at a point the run has
    /// settled on. Only accepted points are remembered: a rejected
    /// step or a Newton iteration would put the past out of order.
    pub(crate) fn remember_delays(&self, t: f64, values: &[f64]) {
        if self.delays.is_empty() {
            return;
        }
        let mut history = self.history.borrow_mut();
        for (delay, trace) in self.delays.iter().zip(history.iter_mut()) {
            let value = delay.source.run(values, t);
            match trace.last() {
                Some((last, _)) if t <= *last + 1e-15 => {}
                _ => trace.push((t, value)),
            }
        }
    }

    /// The longest step that keeps every delay looking at a past the
    /// run has already been through.
    pub(crate) fn delay_step_limit(&self) -> f64 {
        self.delays
            .iter()
            .map(|delay| delay.seconds)
            .fold(f64::INFINITY, f64::min)
    }

    /// Whether the branch each run-time `if` equation was compiled for
    /// is still the branch that holds.
    ///
    /// The event machinery puts the step a hair past the crossing, so
    /// at the point this is asked the relation has already switched and
    /// the answer is unambiguous.
    pub(crate) fn mode_holds(&self, values: &[f64], time: f64) -> bool {
        self.mode_monitor.iter().all(|(conditions, compiled_for)| {
            let now = conditions
                .iter()
                .position(|condition| condition.run(values, time) != 0.0)
                .unwrap_or(conditions.len());
            now == *compiled_for
        })
    }

    /// The stall a segment reports: the point it resumes from is the
    /// last row it *recorded*, never a re-evaluation at the breakdown -
    /// the algebraic layer is exactly what cannot be trusted there.
    pub(crate) fn stall_at_last_row(
        &self,
        columns: Vec<String>,
        rows: Vec<Vec<f64>>,
        method: SolverMethod,
        mode_change: bool,
    ) -> Result<AdaptiveOutcome, SimError> {
        let Some(last) = rows.last() else {
            return err("the run stalled before producing a single point".to_string());
        };
        let mut values: HashMap<String, f64> = self.parameters.iter().cloned().collect();
        for (name, value) in columns.iter().zip(last).skip(1) {
            values.insert(name.clone(), *value);
        }
        let time = last[0];
        Ok(AdaptiveOutcome::Stalled(Stall {
            partial: SimResult {
                columns,
                rows,
                parameters: self.parameters.clone(),
                terminated: None,
                method,
                reselections: 0,
            },
            time,
            values,
            mode_change,
        }))
    }
}
