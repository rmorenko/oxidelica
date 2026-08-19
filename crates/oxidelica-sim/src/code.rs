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
        Expr::Index(base, _) | Expr::Member(base, _) => {
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
                    return crate::walk::walk(programs, name, &vals, ctx.time, ctx.depth + 1);
                }
            }
            match name.as_str() {
                "der" => return err("der() outside a state equation is not supported in M0"),
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
        | Expr::NamedArg(_, _)
        | Expr::Tuple(_) => return err("an array reached the evaluator".to_string()),
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
            Code::Program(walked, name, args) => {
                let given: Vec<f64> = args.iter().map(|arg| arg.run(values, time)).collect();
                match crate::walk::walk(&walked.programs, name, &given, time, 0) {
                    Ok(worth) => worth,
                    Err(SimError(why)) => {
                        if let Ok(mut held) = walked.trouble.lock() {
                            held.get_or_insert(why);
                        }
                        f64::NAN
                    }
                }
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
            Expr::Index(_, _)
            | Expr::Member(_, _)
            | Expr::Array(_)
            | Expr::Elementwise(_, _, _)
            | Expr::Range(_, _, _)
            | Expr::Comprehension(_, _, _)
            | Expr::ColonSubscript
            | Expr::EndSubscript
            | Expr::MatrixRows(_)
            | Expr::NamedArg(_, _)
            | Expr::Tuple(_) => {
                return err("subscripts and arrays survive flattening only as scalars".to_string())
            }
        })
    }

    /// The built-in behind a call, with its arity checked here rather
    /// than on every evaluation.
    pub(crate) fn compile_call(&self, name: &str, args: &[Expr]) -> Result<Code, SimError> {
        // A body this run walks answers to its own name before any
        // built-in rule is looked for.
        if self.walked.programs.contains_key(name) {
            return Ok(Code::Program(
                self.walked.clone(),
                name.to_string(),
                args.iter()
                    .map(|arg| self.compile(arg))
                    .collect::<Result<Vec<_>, SimError>>()?,
            ));
        }
        let unary = match name {
            "ceil" => Some(Unary::Ceil),
            "floor" => Some(Unary::Floor),
            "integer" => Some(Unary::IntegerPart),
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
        let binary = match name {
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
