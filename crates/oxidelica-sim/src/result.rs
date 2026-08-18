//! The table a run produces.

use crate::*;

impl SimResult {
    /// Render the result as CSV text.
    pub fn to_csv(&self) -> String {
        use std::fmt::Write;
        // A large result is mostly numbers, so this writes them straight
        // into one buffer: a `format!` per cell would allocate a string
        // for every value, which on a big model costs more than the
        // simulation that produced them. The values are written at full
        // precision - shortest text that reads back as the same double -
        // rather than padded to a fixed number of decimals.
        let mut out = String::with_capacity(
            self.columns.iter().map(|c| c.len() + 1).sum::<usize>()
                + self.rows.len() * self.columns.len() * 12,
        );
        // A column name may itself hold a comma - `mm[1,2]` - so names
        // are quoted the way CSV quotes things when they need it.
        for (index, name) in self.columns.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            if name.contains(',') || name.contains('"') {
                out.push('"');
                out.push_str(&name.replace('"', "\"\""));
                out.push('"');
            } else {
                out.push_str(name);
            }
        }
        out.push('\n');
        for row in &self.rows {
            for (index, value) in row.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{value}");
            }
            out.push('\n');
        }
        out
    }
}
