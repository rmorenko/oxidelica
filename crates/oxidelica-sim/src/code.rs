//! The evaluator: expressions compiled to slot-based code, and the
//! table of slots they read.

use crate::*;

/// Booleans are carried as 1.0 and 0.0, like everywhere else here.
/// `nthRoot(v, n)`: the n-th root of v. A negative v has one when n is
/// odd, and `powf` will not find it - it gives NaN for every negative
/// base - so the sign is taken out and put back.
pub(crate) fn nth_root(value: f64, n: f64) -> f64 {
    if value < 0.0 && n.fract() == 0.0 && (n / 2.0).fract() != 0.0 {
        -(-value).powf(1.0 / n)
    } else {
        value.powf(1.0 / n)
    }
}

/// What an expression the run cannot carry was, said plainly.
///
/// The refusal without it names the rule and not the thing, and the
/// thing is what says which of several quite different faults this is:
/// a subscript nothing expanded, a whole array where a number was
/// wanted, a field of a record that never came apart.
pub(crate) fn shape_of(expr: &Expr) -> String {
    match expr {
        Expr::Index(base, subscripts) => {
            let names = |e: &Expr| match e {
                Expr::Ref(name) => name.clone(),
                other => shape_of(other),
            };
            let inside: Vec<String> = subscripts.iter().map(names).collect();
            format!("`{}` subscripted by [{}]", names(base), inside.join(", "))
        }
        Expr::Member(base, field) => match base.as_ref() {
            Expr::Ref(name) => format!("`{name}.{field}`, a field of a record"),
            _ => format!("`.{field}`, a field of a record"),
        },
        Expr::Array(items) => format!("an array of {} written out", items.len()),
        Expr::MatrixRows(rows) => format!("a matrix of {} row(s)", rows.len()),
        Expr::Range(..) => "a range".to_string(),
        Expr::Comprehension(..) => "a comprehension".to_string(),
        Expr::Elementwise(..) => "an elementwise operation".to_string(),
        Expr::ColonSubscript => "a `:` subscript".to_string(),
        Expr::EndSubscript => "an `end` subscript".to_string(),
        Expr::NamedArg(name, _) => format!("`{name} = ...`, a named argument"),
        Expr::Tuple(items) => format!("a tuple of {}", items.len()),
        // The rest are the ones a run can carry, and reaching here
        // with one means the fault is elsewhere - so they are listed
        // rather than swept up, and a variant added to `Expr` has to
        // be given a name here rather than quietly becoming "more
        // than one number".
        Expr::Number(_)
        | Expr::Bool(_)
        | Expr::Str(_)
        | Expr::Ref(_)
        | Expr::Time
        | Expr::Call(..)
        | Expr::WithDerivative(..)
        | Expr::Neg(_)
        | Expr::Not(_)
        | Expr::Bin(..)
        | Expr::Rel(..)
        | Expr::And(..)
        | Expr::Or(..)
        | Expr::If(..) => "an expression of more than one number".to_string(),
    }
}

pub(crate) fn truth(yes: bool) -> f64 {
    if yes {
        1.0
    } else {
        0.0
    }
}

/// Booleans are represented as 1.0 / 0.0 (proper typing is an M1+ task).
pub(crate) fn eval(expr: &Expr, ctx: &EvalCtx) -> Result<f64, SimError> {
    use oxidelica_parser::BinOp::*;
    Ok(match expr {
        // A call kept whole for its derivative is worth what it works
        // out to; the rule beside it is for differentiation alone.
        Expr::WithDerivative(value, _, _) => eval(value, ctx)?,
        Expr::Str(text) => {
            return err(format!(
                "`\"{text}\"` is a String, and a String has no value a step can carry; \
                 strings are settled before the run"
            ))
        }
        Expr::Number(n) => *n,
        Expr::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        Expr::Time => ctx.time,
        Expr::Ref(name) => match ctx.vars.get(name) {
            Some(v) => *v,
            None => return err(format!("unknown variable `{name}`")),
        },
        Expr::Neg(inner) => -eval(inner, ctx)?,
        Expr::Not(inner) => {
            if eval(inner, ctx)? == 0.0 {
                1.0
            } else {
                0.0
            }
        }
        Expr::And(l, r) => {
            if eval(l, ctx)? != 0.0 && eval(r, ctx)? != 0.0 {
                1.0
            } else {
                0.0
            }
        }
        Expr::Or(l, r) => {
            if eval(l, ctx)? != 0.0 || eval(r, ctx)? != 0.0 {
                1.0
            } else {
                0.0
            }
        }
        Expr::Rel(op, l, r) => {
            let a = eval(l, ctx)?;
            let b = eval(r, ctx)?;
            let holds = match op {
                RelOp::Lt => a < b,
                RelOp::Le => a <= b,
                RelOp::Gt => a > b,
                RelOp::Ge => a >= b,
                RelOp::Eq => a == b,
                RelOp::Ne => a != b,
            };
            if holds {
                1.0
            } else {
                0.0
            }
        }
        Expr::If(cond, then_branch, else_branch) => {
            if eval(cond, ctx)? != 0.0 {
                eval(then_branch, ctx)?
            } else {
                eval(else_branch, ctx)?
            }
        }
        Expr::Bin(op, l, r) => {
            let a = eval(l, ctx)?;
            let b = eval(r, ctx)?;
            match op {
                Add => a + b,
                Sub => a - b,
                Mul => a * b,
                Div => a / b,
                Pow => a.powf(b),
            }
        }
        // A body written here in Rust may answer with several numbers
        // at once - a generator gives a value and the state it moved
        // to - and each place of that answer is read by subscript.
        // The standard library builds a generator's first state by
        // drawing ten numbers from a seed, so this is a parameter
        // rather than anything the run works out.
        Expr::Index(base, subscripts) => {
            if let (Expr::Call(called, args), [which]) = (base.as_ref(), subscripts.as_slice()) {
                if oxidelica_parser::outside::written_here(called) {
                    // However the declaration grouped them, what the
                    // body takes is the numbers: an array argument is
                    // as many of them as it holds.
                    let mut given = Vec::new();
                    for arg in args {
                        match arg {
                            Expr::Array(items) => {
                                for item in items {
                                    given.push(eval(item, ctx)?);
                                }
                            }
                            one => given.push(eval(one, ctx)?),
                        }
                    }
                    let place = eval(which, ctx)? as usize;
                    let answer = oxidelica_parser::outside::answer(called, &given)
                        .and_then(|answer| answer.get(place.checked_sub(1)?).copied());
                    return answer.ok_or_else(|| {
                        SimError(format!(
                            "`{called}` was handed {} number(s) and asked for place {place}",
                            given.len()
                        ))
                    });
                }
            }
            // And a body the run carries may answer with a record,
            // whose fields are read the same way: the water of the
            // library starts a tank at `waterBaseProp_pT(p, T, 0)[5]`,
            // the fifth field of the properties it answers with. The
            // compiled side of the run has this door already; this is
            // the same door on the side that evaluates.
            if let (Expr::Call(called, args), [which]) = (base.as_ref(), subscripts.as_slice()) {
                if let Some(programs) = ctx.programs {
                    if programs.contains_key(called.as_str()) {
                        // A walk counts its arguments by the lengths
                        // it was handed - one entry per argument, zero
                        // for a scalar - rather than by how many
                        // numbers arrived.
                        let (mut given, mut lengths) = (Vec::new(), Vec::new());
                        for arg in args {
                            match arg {
                                Expr::Array(items) => {
                                    // A table is rows of numbers: every
                                    // element goes over in order and
                                    // both dimensions are said, so the
                                    // body can put it back together.
                                    let rows: Option<Vec<&Vec<Expr>>> = items
                                        .iter()
                                        .map(|item| match item {
                                            Expr::Array(row) => Some(row),
                                            _ => None,
                                        })
                                        .collect();
                                    match rows.filter(|rows| {
                                        rows.first().is_some_and(|first| {
                                            rows.iter().all(|row| row.len() == first.len())
                                        })
                                    }) {
                                        Some(rows) => {
                                            let width = rows[0].len();
                                            for row in &rows {
                                                for item in row.iter() {
                                                    given.push(eval(item, ctx)?);
                                                }
                                            }
                                            lengths.push(vec![rows.len(), width]);
                                        }
                                        None => {
                                            for item in items {
                                                given.push(eval(item, ctx)?);
                                            }
                                            lengths.push(vec![items.len()]);
                                        }
                                    }
                                }
                                one => {
                                    given.push(eval(one, ctx)?);
                                    lengths.push(Vec::new());
                                }
                            }
                        }
                        let place = eval(which, ctx)? as usize;
                        let answer = crate::walk::walk(
                            programs,
                            called,
                            &given,
                            &lengths,
                            ctx.time,
                            ctx.depth + 1,
                        )?;
                        return place
                            .checked_sub(1)
                            .and_then(|at| answer.get(at))
                            .copied()
                            .ok_or_else(|| {
                                SimError(format!(
                                    "`{called}` answered with {} number(s) and place \
                                     {place} was asked for",
                                    answer.len()
                                ))
                            });
                    }
                }
            }
            return err(format!(
                "unresolved array subscript on {base:?}: flattening should have expanded it"
            ));
        }
        Expr::Member(base, _) => {
            return err(format!(
                "unresolved array subscript on {base:?}: flattening should have expanded it"
            ))
        }
        Expr::Call(name, args) => {
            let vals: Result<Vec<f64>, SimError> = args.iter().map(|a| eval(a, ctx)).collect();
            let vals = vals?;
            let arity = |n: usize| -> Result<(), SimError> {
                if vals.len() == n {
                    Ok(())
                } else {
                    err(format!(
                        "{name}: expects {n} argument(s), got {}",
                        vals.len()
                    ))
                }
            };
            // A body the run walks answers before any built-in rule is
            // looked for: it is the model's own function, and its name
            // is its own.
            if let Some(programs) = ctx.programs {
                if programs.contains_key(name.as_str()) {
                    // Inside a walked body one call hands another
                    // numbers it has already worked out, and a name
                    // that stands for several is written out as an
                    // array before it gets here.
                    let lengths = vec![Vec::new(); vals.len()];
                    // A call inside a walked body asks for the one
                    // number a body written that way answers with.
                    return crate::walk::walk(
                        programs,
                        name,
                        &vals,
                        &lengths,
                        ctx.time,
                        ctx.depth + 1,
                    )
                    .and_then(|answer| {
                        answer
                            .first()
                            .copied()
                            .ok_or_else(|| SimError(format!("`{name}` gave nothing back")))
                    });
                }
            }
            match operator_name(name) {
                "der" => return err("der() outside a state equation is not supported in M0"),
                // Whether the run has begun. Asked here rather than of
                // the event machinery because choosing the branch of
                // an `if` equation happens before that machinery
                // exists, and what holds for every instant of the run
                // is that the initial one is over: the branch guarded
                // by `initial()` says what a value was for no length
                // of time. This is how the hysteresis models give a
                // state something to start from.
                "initial" | "terminal" => 0.0,
                // The ordinal of an enumeration value, which is what an
                // enumeration is carried as.
                "Integer" => {
                    arity(1)?;
                    vals[0]
                }
                "sin" => {
                    arity(1)?;
                    vals[0].sin()
                }
                "cos" => {
                    arity(1)?;
                    vals[0].cos()
                }
                "tan" => {
                    arity(1)?;
                    vals[0].tan()
                }
                "asin" => {
                    arity(1)?;
                    vals[0].asin()
                }
                "acos" => {
                    arity(1)?;
                    vals[0].acos()
                }
                "atan" => {
                    arity(1)?;
                    vals[0].atan()
                }
                "atan2" => {
                    arity(2)?;
                    vals[0].atan2(vals[1])
                }
                "nthRoot" => {
                    arity(2)?;
                    nth_root(vals[0], vals[1])
                }
                "sinh" => {
                    arity(1)?;
                    vals[0].sinh()
                }
                "cosh" => {
                    arity(1)?;
                    vals[0].cosh()
                }
                "tanh" => {
                    arity(1)?;
                    vals[0].tanh()
                }
                "exp" => {
                    arity(1)?;
                    vals[0].exp()
                }
                "log" => {
                    arity(1)?;
                    vals[0].ln()
                }
                "log10" => {
                    arity(1)?;
                    vals[0].log10()
                }
                "sqrt" => {
                    arity(1)?;
                    vals[0].sqrt()
                }
                "abs" => {
                    arity(1)?;
                    vals[0].abs()
                }
                "sign" => {
                    arity(1)?;
                    vals[0].signum()
                }
                "min" => {
                    arity(2)?;
                    vals[0].min(vals[1])
                }
                "max" => {
                    arity(2)?;
                    vals[0].max(vals[1])
                }
                "ceil" => {
                    arity(1)?;
                    vals[0].ceil()
                }
                "floor" | "integer" => {
                    arity(1)?;
                    vals[0].floor()
                }
                "div" => {
                    arity(2)?;
                    (vals[0] / vals[1]).trunc()
                }
                "mod" => {
                    arity(2)?;
                    vals[0] - (vals[0] / vals[1]).floor() * vals[1]
                }
                "rem" => {
                    arity(2)?;
                    vals[0] - (vals[0] / vals[1]).trunc() * vals[1]
                }
                other => return err(format!("unknown function `{other}`")),
            }
        }
        // Arrays are expanded into scalars while flattening, so one
        // here would mean the compiler let something through.
        Expr::Array(_)
        | Expr::Elementwise(_, _, _)
        | Expr::Range(_, _, _)
        | Expr::Comprehension(_, _, _)
        | Expr::ColonSubscript
        | Expr::EndSubscript
        | Expr::MatrixRows(_)
        | Expr::Tuple(_) => return err("an array reached the evaluator".to_string()),
        // A named argument is its value under a name: the water of the
        // library calls its property function with `phase = 0`, and by
        // the time a parameter is settled the name has done its work -
        // the argument sits in the seat the callee declared.
        Expr::NamedArg(_, value) => return eval(value, ctx),
    })
}

impl Code {
    /// Evaluate against the value array of a run. Nothing here can
    /// fail: unknown names and unknown functions were rejected while
    /// compiling, which is why this returns a number rather than a
    /// result.
    pub(crate) fn run(&self, values: &[f64], time: f64) -> f64 {
        match self {
            Code::Const(value) => *value,
            // A body the run walks: work the arguments out, walk it,
            // and where the walk fails leave the reason behind and
            // answer with a number that is not one.
            Code::Program(walked, name, args, lengths, which) => {
                let given: Vec<f64> = args.iter().map(|arg| arg.run(values, time)).collect();
                // Which number of the answer this stands for was
                // settled where the call was laid out, against what the
                // body declares it answers with.
                match crate::walk::walk(&walked.programs, name, &given, lengths, time, 0)
                    .map(|answer| answer[*which])
                {
                    Ok(worth) => worth,
                    Err(SimError(why)) => {
                        if let Ok(mut held) = walked.trouble.lock() {
                            held.get_or_insert(why);
                        }
                        f64::NAN
                    }
                }
            }
            // A body written here in Rust: the numbers in, the
            // numbers out, and the place of the answer this stands
            // for. The shape was checked where the call was laid out.
            Code::Outside(called, args, which) => {
                let given: Vec<f64> = args.iter().map(|arg| arg.run(values, time)).collect();
                oxidelica_parser::outside::answer(called, &given)
                    .and_then(|answer| answer.get(*which).copied())
                    .unwrap_or(f64::NAN)
            }
            Code::Slot(slot) => values[*slot],
            Code::Time => time,
            Code::Neg(inner) => -inner.run(values, time),
            Code::Not(inner) => truth(inner.run(values, time) == 0.0),
            Code::Bin(op, l, r) => {
                let (a, b) = (l.run(values, time), r.run(values, time));
                match op {
                    BinOp::Add => a + b,
                    BinOp::Sub => a - b,
                    BinOp::Mul => a * b,
                    BinOp::Div => a / b,
                    BinOp::Pow => a.powf(b),
                }
            }
            Code::Rel(op, l, r) => {
                let (a, b) = (l.run(values, time), r.run(values, time));
                truth(match op {
                    RelOp::Lt => a < b,
                    RelOp::Le => a <= b,
                    RelOp::Gt => a > b,
                    RelOp::Ge => a >= b,
                    RelOp::Eq => a == b,
                    RelOp::Ne => a != b,
                })
            }
            Code::And(l, r) => truth(l.run(values, time) != 0.0 && r.run(values, time) != 0.0),
            Code::Or(l, r) => truth(l.run(values, time) != 0.0 || r.run(values, time) != 0.0),
            Code::If(condition, then, otherwise) => {
                if condition.run(values, time) != 0.0 {
                    then.run(values, time)
                } else {
                    otherwise.run(values, time)
                }
            }
            Code::Unary(function, argument) => {
                let x = argument.run(values, time);
                match function {
                    Unary::Ceil => x.ceil(),
                    Unary::Floor => x.floor(),
                    // integer(x) truncates toward negative infinity,
                    // like floor - the spec defines it that way.
                    Unary::IntegerPart => x.floor(),
                    // `Integer(e)` is the ordinal of an enumeration
                    // value, which is what one is carried as: there is
                    // nothing left to do to it. It is not `integer(x)`,
                    // which cuts a number down.
                    Unary::Ordinal => x,
                    Unary::Sin => x.sin(),
                    Unary::Cos => x.cos(),
                    Unary::Tan => x.tan(),
                    Unary::Asin => x.asin(),
                    Unary::Acos => x.acos(),
                    Unary::Atan => x.atan(),
                    Unary::Sinh => x.sinh(),
                    Unary::Cosh => x.cosh(),
                    Unary::Tanh => x.tanh(),
                    Unary::Exp => x.exp(),
                    Unary::Log => x.ln(),
                    Unary::Log10 => x.log10(),
                    Unary::Sqrt => x.sqrt(),
                    Unary::Abs => x.abs(),
                    Unary::Sign => x.signum(),
                }
            }
            Code::Binary(function, l, r) => {
                let (a, b) = (l.run(values, time), r.run(values, time));
                match function {
                    Binary::Atan2 => a.atan2(b),
                    Binary::NthRoot => nth_root(a, b),
                    Binary::Min => a.min(b),
                    Binary::Max => a.max(b),
                    // Integer division truncates toward zero; mod and
                    // rem follow their spec definitions from it.
                    Binary::Div => (a / b).trunc(),
                    Binary::Mod => a - (a / b).floor() * b,
                    Binary::Rem => a - (a / b).trunc() * b,
                }
            }
        }
    }
}

impl SlotTable {
    /// An empty table.
    pub(crate) fn new(programs: HashMap<String, ClassDef>) -> SlotTable {
        SlotTable {
            index: HashMap::new(),
            template: Vec::new(),
            walked: std::sync::Arc::new(Walked {
                programs,
                trouble: std::sync::Mutex::new(None),
            }),
        }
    }

    /// Give a name a slot, or return the one it already has.
    pub(crate) fn slot(&mut self, name: &str) -> Slot {
        if let Some(slot) = self.index.get(name) {
            return *slot;
        }
        let slot = self.template.len();
        self.template.push(0.0);
        self.index.insert(name.to_string(), slot);
        slot
    }

    /// Give a name a slot holding a value that never changes.
    pub(crate) fn constant(&mut self, name: &str, value: f64) -> Slot {
        let slot = self.slot(name);
        self.template[slot] = value;
        slot
    }

    /// Resolve an expression into code, refusing anything that names a
    /// variable or a function the model does not have.
    pub(crate) fn compile(&self, expr: &Expr) -> Result<Code, SimError> {
        Ok(match expr {
            Expr::WithDerivative(value, _, _) => self.compile(value)?,
            Expr::Str(text) => {
                return err(format!(
                    "`\"{text}\"` is a String, and a String has no value a step can carry; \
                     strings are settled before the run"
                ))
            }
            Expr::Number(value) => Code::Const(*value),
            Expr::Bool(value) => Code::Const(truth(*value)),
            Expr::Time => Code::Time,
            Expr::Ref(name) => match self.index.get(name) {
                Some(slot) => Code::Slot(*slot),
                None => return err(format!("unknown variable `{name}`")),
            },
            Expr::Neg(inner) => Code::Neg(Box::new(self.compile(inner)?)),
            Expr::Not(inner) => Code::Not(Box::new(self.compile(inner)?)),
            Expr::Bin(op, l, r) => {
                Code::Bin(*op, Box::new(self.compile(l)?), Box::new(self.compile(r)?))
            }
            Expr::Rel(op, l, r) => {
                Code::Rel(*op, Box::new(self.compile(l)?), Box::new(self.compile(r)?))
            }
            Expr::And(l, r) => Code::And(Box::new(self.compile(l)?), Box::new(self.compile(r)?)),
            Expr::Or(l, r) => Code::Or(Box::new(self.compile(l)?), Box::new(self.compile(r)?)),
            Expr::If(condition, then, otherwise) => Code::If(
                Box::new(self.compile(condition)?),
                Box::new(self.compile(then)?),
                Box::new(self.compile(otherwise)?),
            ),
            Expr::Call(name, args) => self.compile_call(name, args)?,
            // `f(x)[2]` of a body the run walks: the same call, asked
            // for the second number of what it answers with. That is
            // how a body answering with an array reaches a model, which
            // holds one number per name.
            Expr::Index(base, subscripts) => match (base.as_ref(), subscripts.as_slice()) {
                // `f(x)[2]` of a body written here: the same call,
                // asked for the second number of what it answers with.
                (Expr::Call(name, args), [Expr::Number(which)])
                    if oxidelica_parser::outside::written_here(name) && *which >= 1.0 =>
                {
                    match self.compile_call(name, args)? {
                        Code::Outside(name, given, _) => {
                            Code::Outside(name, given, *which as usize - 1)
                        }
                        other => other,
                    }
                }
                (Expr::Call(name, args), [Expr::Number(which)])
                    if self.walked.programs.contains_key(name) && *which >= 1.0 =>
                {
                    match self.compile_call(name, args)? {
                        Code::Program(walked, name, given, lengths, _) => {
                            Code::Program(walked, name, given, lengths, *which as usize - 1)
                        }
                        other => other,
                    }
                }
                _ => {
                    return err(format!(
                        "subscripts and arrays survive flattening only as scalars: {}",
                        shape_of(expr)
                    ));
                }
            },
            Expr::Member(_, _)
            | Expr::Array(_)
            | Expr::Elementwise(_, _, _)
            | Expr::Range(_, _, _)
            | Expr::Comprehension(_, _, _)
            | Expr::ColonSubscript
            | Expr::EndSubscript
            | Expr::MatrixRows(_)
            | Expr::NamedArg(_, _)
            | Expr::Tuple(_) => {
                return err(format!(
                    "subscripts and arrays survive flattening only as scalars: {}",
                    shape_of(expr)
                ))
            }
        })
    }

    /// The built-in behind a call, with its arity checked here rather
    /// than on every evaluation.
    pub(crate) fn compile_call(&self, name: &str, args: &[Expr]) -> Result<Code, SimError> {
        // A body written here in Rust answers to the name it is called
        // by outside. An array argument was handed over written out,
        // and the body takes the numbers in the order they were
        // written, so there is nothing to keep about the grouping.
        if oxidelica_parser::outside::written_here(name) {
            // A matrix arrives as an array of its rows, and what the
            // body takes is the numbers themselves, in the order they
            // were written.
            fn leaves<'a>(expr: &'a Expr, out: &mut Vec<&'a Expr>) {
                match expr {
                    Expr::Array(items) => items.iter().for_each(|item| leaves(item, out)),
                    one => out.push(one),
                }
            }
            let (mut given, mut handed) = (Vec::new(), Vec::new());
            for arg in args {
                let mut here = Vec::new();
                leaves(arg, &mut here);
                handed.push(here.len());
                for one in here {
                    given.push(self.compile(one)?);
                }
            }
            let Some((takes, _)) = oxidelica_parser::outside::shape(name, &handed) else {
                return err(format!(
                    "`{name}` is written here, and not for what it was handed: {} number(s)",
                    given.len()
                ));
            };
            if given.len() != takes {
                return err(format!(
                    "`{name}` takes {takes} number(s), and it was handed {}",
                    given.len()
                ));
            }
            return Ok(Code::Outside(name.to_string(), given, 0));
        }
        // A body this run walks answers to its own name before any
        // built-in rule is looked for.
        if self.walked.programs.contains_key(name) {
            // An argument written out as an array is handed over as its
            // elements, and how many there were is kept so the body can
            // put them back together under the name it knows.
            let (mut given, mut lengths) = (Vec::new(), Vec::new());
            for arg in args {
                match arg {
                    Expr::Array(items) => {
                        // Rows of equal length are a table and go over
                        // as one, both dimensions said; anything else
                        // is a plain list.
                        let rows: Option<Vec<&Vec<Expr>>> = items
                            .iter()
                            .map(|item| match item {
                                Expr::Array(row) => Some(row),
                                _ => None,
                            })
                            .collect();
                        match rows.filter(|rows| {
                            rows.first()
                                .is_some_and(|first| rows.iter().all(|row| row.len() == first.len()))
                        }) {
                            Some(rows) => {
                                let width = rows[0].len();
                                for row in &rows {
                                    for item in row.iter() {
                                        given.push(self.compile(item)?);
                                    }
                                }
                                lengths.push(vec![rows.len(), width]);
                            }
                            None => {
                                for item in items {
                                    given.push(self.compile(item)?);
                                }
                                lengths.push(vec![items.len()]);
                            }
                        }
                    }
                    one => {
                        given.push(self.compile(one)?);
                        lengths.push(Vec::new());
                    }
                }
            }
            return Ok(Code::Program(
                self.walked.clone(),
                name.to_string(),
                given,
                lengths,
                0,
            ));
        }
        let unary = match operator_name(name) {
            "ceil" => Some(Unary::Ceil),
            "floor" => Some(Unary::Floor),
            "integer" => Some(Unary::IntegerPart),
            "Integer" => Some(Unary::Ordinal),
            "sin" => Some(Unary::Sin),
            "cos" => Some(Unary::Cos),
            "tan" => Some(Unary::Tan),
            "asin" => Some(Unary::Asin),
            "acos" => Some(Unary::Acos),
            "atan" => Some(Unary::Atan),
            "sinh" => Some(Unary::Sinh),
            "cosh" => Some(Unary::Cosh),
            "tanh" => Some(Unary::Tanh),
            "exp" => Some(Unary::Exp),
            "log" => Some(Unary::Log),
            "log10" => Some(Unary::Log10),
            "sqrt" => Some(Unary::Sqrt),
            "abs" => Some(Unary::Abs),
            "sign" => Some(Unary::Sign),
            _ => None,
        };
        if let Some(function) = unary {
            if args.len() != 1 {
                return err(format!("{name}: expects 1 argument, got {}", args.len()));
            }
            return Ok(Code::Unary(function, Box::new(self.compile(&args[0])?)));
        }
        let binary = match operator_name(name) {
            "atan2" => Some(Binary::Atan2),
            "nthRoot" => Some(Binary::NthRoot),
            "min" => Some(Binary::Min),
            "max" => Some(Binary::Max),
            "div" => Some(Binary::Div),
            "mod" => Some(Binary::Mod),
            "rem" => Some(Binary::Rem),
            _ => None,
        };
        if let Some(function) = binary {
            if args.len() != 2 {
                return err(format!("{name}: expects 2 arguments, got {}", args.len()));
            }
            return Ok(Code::Binary(
                function,
                Box::new(self.compile(&args[0])?),
                Box::new(self.compile(&args[1])?),
            ));
        }
        if name == "der" {
            return err("der() outside a state equation is not supported".to_string());
        }
        err(format!("unknown function `{name}`"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every construction the refusal may have to name, named.
    ///
    /// These reach the reader only through a model that got that far,
    /// and most of them cannot survive flattening at all - a `:`
    /// subscript is resolved long before, a comprehension is unrolled -
    /// so what is checked here is the wording rather than a route
    /// through the compiler.
    #[test]
    fn a_construction_that_is_not_a_scalar_says_what_it_is() {
        let name = |text: &str| Expr::Ref(text.to_string());
        let one = || Box::new(Expr::Number(1.0));
        for (expr, expected) in [
            (
                Expr::Index(Box::new(name("v")), vec![Expr::Number(2.0)]),
                "`v` subscripted by",
            ),
            (
                Expr::Member(Box::new(name("r")), "field".to_string()),
                "`r.field`, a field of a record",
            ),
            (
                Expr::Member(Box::new(Expr::Number(1.0)), "field".to_string()),
                "`.field`, a field of a record",
            ),
            (Expr::Array(vec![Expr::Number(1.0)]), "an array of 1"),
            (
                Expr::MatrixRows(vec![vec![Expr::Number(1.0)]]),
                "a matrix of 1 row(s)",
            ),
            (Expr::Range(one(), None, one()), "a range"),
            (
                Expr::Comprehension(one(), "i".to_string(), one()),
                "a comprehension",
            ),
            (
                Expr::Elementwise(oxidelica_parser::BinOp::Add, one(), one()),
                "an elementwise operation",
            ),
            (Expr::ColonSubscript, "a `:` subscript"),
            (Expr::EndSubscript, "an `end` subscript"),
            (
                Expr::NamedArg("actual".to_string(), one()),
                "`actual = ...`, a named argument",
            ),
            (Expr::Tuple(vec![Some(Expr::Number(1.0))]), "a tuple of 1"),
            (Expr::Time, "an expression of more than one number"),
        ] {
            let said = shape_of(&expr);
            assert!(said.contains(expected), "{said} does not say {expected}");
        }
        // A subscript that is itself a construction is named the same
        // way, one level down.
        let nested = shape_of(&Expr::Index(
            Box::new(Expr::Array(vec![Expr::Number(1.0)])),
            vec![Expr::ColonSubscript],
        ));
        assert!(nested.contains("a `:` subscript"), "{nested}");
    }
}

#[cfg(test)]
mod outside_places {
    use super::*;

    /// A body written here answers several numbers, and each place of
    /// that answer is read by subscript. Asked for a place it does
    /// not have, it says which name it was and what it was handed.
    #[test]
    fn a_place_past_the_answer_is_refused_by_name() {
        let ctx = EvalCtx {
            vars: &HashMap::new(),
            time: 0.0,
            programs: None,
            depth: 0,
        };
        let call = |place: f64| {
            Expr::Index(
                Box::new(Expr::Call(
                    "ModelicaRandom_xorshift64star".to_string(),
                    vec![Expr::Array(vec![Expr::Number(1.0), Expr::Number(0.0)])],
                )),
                vec![Expr::Number(place)],
            )
        };
        // Three places: the value drawn and the two halves of the
        // state it moved to.
        assert!(eval(&call(1.0), &ctx).is_ok());
        assert!(eval(&call(3.0), &ctx).is_ok());
        let why = eval(&call(4.0), &ctx).unwrap_err().0;
        assert!(why.contains("xorshift64star"), "{why}");
        assert!(why.contains("place 4"), "{why}");
        // A subscript of nothing is past the answer the other way.
        assert!(eval(&call(0.0), &ctx).is_err());
        // An argument that is not settled leaves the refusal to the
        // layer that knows the name.
        let unsettled = Expr::Index(
            Box::new(Expr::Call(
                "ModelicaRandom_xorshift64star".to_string(),
                vec![Expr::Ref("nowhere".to_string())],
            )),
            vec![Expr::Number(1.0)],
        );
        assert!(eval(&unsettled, &ctx).is_err());
        // A member is not a place of an answer and keeps its own
        // refusal.
        let member = Expr::Member(Box::new(Expr::Ref("r".to_string())), "x".to_string());
        assert!(eval(&member, &ctx).is_err());
        // A name nobody here answers for keeps the old refusal.
        let elsewhere = Expr::Index(
            Box::new(Expr::Call("nobody".to_string(), vec![])),
            vec![Expr::Number(1.0)],
        );
        let why = eval(&elsewhere, &ctx).unwrap_err().0;
        assert!(why.contains("unresolved array subscript"), "{why}");
    }
}
