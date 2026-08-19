//! The equation section: ordinary equations, `for` and `if` equations,
//! `connect`, `when`, and the clauses that draw a state machine or an
//! overconstrained graph.

use super::*;

impl Parser {
    pub(super) fn equation_item(&mut self) -> Result<EquationItem, ParseError> {
        match self.equation_line()? {
            EquationLine::Equation(equation) => Ok(equation),
            EquationLine::Call(call) => Err(self.err(format!(
                "`{}` stands on its own where an equation is wanted",
                sketch_call(&call)
            ))),
        }
    }

    /// One line of an equation section: an equation, or a call standing
    /// on its own.
    ///
    /// Nothing receives such a call's outputs, so what it is written
    /// for is the checks its body makes - the standard library's fluid
    /// boundaries say `checkBoundary(...)` among their equations and
    /// take nothing back from it.
    pub(super) fn equation_line(&mut self) -> Result<EquationLine, ParseError> {
        // `(a, b) = f(...)` fills several targets from one call. Only
        // a top-level comma tells it from a parenthesised expression,
        // so the tuple is tried first and abandoned without a trace.
        let lhs = match self.tuple_targets() {
            Some(targets) => Expr::Tuple(targets),
            None => self.expr()?,
        };
        if self.peek() != &Token::Assign {
            let Expr::Call(..) = &lhs else {
                return Err(self.err(format!(
                    "expected `=` in an equation, found `{}`",
                    self.peek()
                )));
            };
            self.opt_string();
            if self.peek() == &Token::Annotation {
                self.annotation_body(&mut Annotated::default())?;
            }
            self.expect(&Token::Semi, "semicolon after the call")?;
            return Ok(EquationLine::Call(lhs));
        }
        self.expect(&Token::Assign, "`=` in equation")?;
        let rhs = self.expr()?;
        self.opt_string();
        if self.peek() == &Token::Annotation {
            self.annotation_body(&mut Annotated::default())?;
        }
        self.expect(&Token::Semi, "semicolon after equation")?;
        Ok(EquationLine::Equation(EquationItem {
            lhs,
            rhs,
            origin: String::new(),
        }))
    }

    /// Try to read `(a, , c)` followed by `=`: the targets of a tuple
    /// equation, `None` for a skipped slot. Anything else - no opening
    /// parenthesis, no top-level comma, no `=` after - restores the
    /// position and returns `None`.
    pub(super) fn tuple_targets(&mut self) -> Option<Vec<Option<Expr>>> {
        if self.peek() != &Token::LParen {
            return None;
        }
        let saved = self.pos;
        self.bump();
        let mut targets = Vec::new();
        let mut saw_comma = false;
        loop {
            match self.peek() {
                Token::Comma | Token::RParen => targets.push(None),
                _ => match self.expr() {
                    Ok(target) => targets.push(Some(target)),
                    Err(_) => {
                        self.pos = saved;
                        return None;
                    }
                },
            }
            match self.peek() {
                Token::Comma => {
                    self.bump();
                    saw_comma = true;
                }
                Token::RParen => {
                    self.bump();
                    break;
                }
                _ => {
                    self.pos = saved;
                    return None;
                }
            }
        }
        if !saw_comma || self.peek() != &Token::Assign {
            self.pos = saved;
            return None;
        }
        Some(targets)
    }

    /// `i in 1:3, j in {1, 5}, k` — the indices of one loop, each with
    /// its own range or with none, where the body is left to say what
    /// it runs over.
    pub(super) fn for_indices(&mut self) -> Result<Vec<(String, Option<Expr>)>, ParseError> {
        let mut indices = Vec::new();
        loop {
            let variable = self.ident("loop variable")?;
            let range = match self.peek() {
                Token::In => {
                    self.bump();
                    Some(self.expr()?)
                }
                _ => None,
            };
            indices.push((variable, range));
            if self.peek() != &Token::Comma {
                return Ok(indices);
            }
            self.bump();
        }
    }

    /// `for <indices> loop <equations> end for;`
    ///
    /// Several indices are the same thing as loops one inside another,
    /// which is what they are turned into: the body belongs to the last
    /// of them, and each earlier one holds the next.
    pub(super) fn for_equation(&mut self) -> Result<ForEquation, ParseError> {
        self.expect(&Token::For, "for")?;
        let indices = self.for_indices()?;
        self.expect(&Token::Loop, "loop after the range")?;
        let mut body = Vec::new();
        while self.peek() != &Token::End {
            match self.peek() {
                Token::Eof => return Err(self.err("unterminated for equation".into())),
                Token::For => body.push(ForBody::Nested(self.for_equation()?)),
                Token::Connect => {
                    let (a, b) = self.connect_clause()?;
                    body.push(ForBody::Connect(a, b));
                }
                // A check written inside a loop is one per round.
                Token::Ident(name) if name == "assert" && self.peek_at(1) == &Token::LParen => {
                    self.bump();
                    let held = self.assert_arguments()?;
                    self.expect(&Token::Semi, "semicolon after assert")?;
                    if let Some((condition, message)) = held {
                        body.push(ForBody::Assert(condition, message));
                    }
                }
                _ => body.push(ForBody::Equation(self.equation_item()?)),
            }
        }
        self.expect(&Token::End, "end for")?;
        self.expect(&Token::For, "for after end")?;
        self.expect(&Token::Semi, "semicolon after end for")?;
        if body.is_empty() {
            return Err(self.err("for equation has no body".into()));
        }
        let mut built: Option<ForEquation> = None;
        for (variable, range) in indices.into_iter().rev() {
            built = Some(ForEquation {
                variable,
                range,
                body: match built {
                    Some(inner) => vec![ForBody::Nested(inner)],
                    None => std::mem::take(&mut body),
                },
            });
        }
        Ok(built.expect("a loop has at least one index"))
    }

    /// `if <cond> then <equations> [elseif …] [else …] end if;` in an
    /// equation section.
    pub(super) fn if_equation(&mut self) -> Result<IfEquation, ParseError> {
        self.expect(&Token::If, "if")?;
        let mut branches = Vec::new();
        // Whether the `if` said anything at all, warning-level checks
        // included: they are read and dropped, so a branch that held
        // only those leaves nothing behind and is still not a mistake.
        let mut said_something = false;
        loop {
            let condition = self.expr()?;
            self.expect(&Token::Then, "then after the condition of an if equation")?;
            let body = self.branch_body()?;
            said_something |= body.dropped > 0;
            branches.extend(flatten_branch(Some(condition), body));
            match self.peek() {
                Token::ElseIf => {
                    self.bump();
                }
                Token::Else => {
                    self.bump();
                    let body = self.branch_body()?;
                    said_something |= body.dropped > 0;
                    branches.extend(flatten_branch(None, body));
                    break;
                }
                _ => break,
            }
        }
        self.expect(&Token::End, "end if")?;
        self.expect(&Token::If, "if after end")?;
        self.expect(&Token::Semi, "semicolon after end if")?;
        if !said_something
            && branches.iter().all(|b| {
                b.equations.is_empty()
                    && b.connects.is_empty()
                    && b.asserts.is_empty()
                    && b.loops.is_empty()
            })
        {
            return Err(self.err("if equation has no equations".into()));
        }
        Ok(IfEquation { branches })
    }

    /// What one branch holds, up to the next `elseif`, `else` or `end`:
    /// equations, `connect` statements, and the `if` equations written
    /// among them.
    pub(super) fn branch_body(&mut self) -> Result<BranchBody, ParseError> {
        let mut equations = Vec::new();
        let mut connects = Vec::new();
        let mut nested = Vec::new();
        let mut asserts = Vec::new();
        let mut loops = Vec::new();
        let mut dropped = 0;
        loop {
            match self.peek() {
                Token::ElseIf | Token::Else | Token::End => break,
                Token::Eof => return Err(self.err("unterminated if equation".into())),
                Token::Connect => connects.push(self.connect_clause()?),
                Token::If => nested.push(self.if_equation()?),
                Token::For => loops.push(self.for_equation()?),
                // A check written among the equations of a branch: it
                // is one of them for the purpose of being read here,
                // and none of them for the purpose of counting.
                Token::Ident(name) if name == "assert" && self.peek_at(1) == &Token::LParen => {
                    self.bump();
                    match self.assert_arguments()? {
                        Some(held) => asserts.push(held),
                        None => dropped += 1,
                    }
                    self.expect(&Token::Semi, "semicolon after assert")?;
                }
                _ => equations.push(self.equation_item()?),
            }
        }
        Ok(BranchBody {
            equations,
            connects,
            nested,
            asserts,
            loops,
            dropped,
        })
    }

    /// `connect(a.b, c.d) annotation(...);` — the references may carry
    /// subscripts (`pins[i]`, `a[2].p`) or name whole arrays.
    pub(super) fn connect_clause(&mut self) -> Result<(Expr, Expr), ParseError> {
        self.expect(&Token::Connect, "connect")?;
        self.expect(&Token::LParen, "parenthesis after connect")?;
        let left = self.connect_ref()?;
        self.expect(&Token::Comma, "comma in connect")?;
        let right = self.connect_ref()?;
        self.expect(&Token::RParen, "closing parenthesis of connect")?;
        if self.peek() == &Token::Annotation {
            self.annotation_body(&mut Annotated::default())?;
        }
        self.expect(&Token::Semi, "semicolon after connect")?;
        Ok((left, right))
    }

    /// One side of a `connect`: a dotted name, optionally subscripted,
    /// optionally followed by more of the path.
    pub(super) fn connect_ref(&mut self) -> Result<Expr, ParseError> {
        let name = self.component_ref()?;
        if self.peek() != &Token::LBracket {
            return Ok(Expr::Ref(name));
        }
        self.bump();
        let mut subscripts = Vec::new();
        loop {
            subscripts.push(self.subscript()?);
            match self.bump() {
                Token::Comma => continue,
                Token::RBracket => break,
                other => {
                    return Err(self.err(format!(
                        "expected `,` or `]` in a subscript, found `{other}`"
                    )))
                }
            }
        }
        let indexed = Expr::Index(Box::new(Expr::Ref(name)), subscripts);
        if self.peek() == &Token::Dot {
            self.bump();
            let path = self.component_ref()?;
            return Ok(Expr::Member(Box::new(indexed), path));
        }
        Ok(indexed)
    }

    /// `when <cond> then <action>; … [elsewhen <cond> then …] end when;`
    ///
    /// A branch holds equations for discrete variables (`x = expr`),
    /// `reinit(state, expr)` and `terminate("message")`.
    pub(super) fn when_clause(&mut self) -> Result<WhenClause, ParseError> {
        self.expect(&Token::When, "when")?;
        let mut branches = Vec::new();
        loop {
            let condition = self.expr()?;
            self.expect(&Token::Then, "then after when condition")?;
            let actions = self.when_actions()?;
            if actions.is_empty() {
                return Err(self.err("when branch has no actions".into()));
            }
            // `when {c1, c2} then ...` fires whenever any one of them
            // becomes true - which is a branch apiece, since a
            // disjunction has no edge of its own once one holds.
            match condition {
                Expr::Array(conditions) if !conditions.is_empty() => {
                    for condition in conditions {
                        branches.push(WhenBranch {
                            condition,
                            actions: actions.clone(),
                        });
                    }
                }
                condition => branches.push(WhenBranch { condition, actions }),
            }
            if self.peek() == &Token::ElseWhen {
                self.bump();
                continue;
            }
            break;
        }
        self.expect(&Token::End, "end when")?;
        self.expect(&Token::When, "when after end")?;
        self.expect(&Token::Semi, "semicolon after end when")?;
        Ok(WhenClause { branches })
    }

    /// The body of one `when` branch, up to `elsewhen` or `end`.
    pub(super) fn when_actions(&mut self) -> Result<Vec<WhenAction>, ParseError> {
        let mut actions = Vec::new();
        loop {
            match self.peek() {
                Token::End | Token::ElseWhen => break,
                Token::Eof => return Err(self.err("unterminated when clause".into())),
                _ => {}
            }
            // A loop of assignments, one per round.
            if self.peek() == &Token::For {
                actions.push(WhenAction::Loop(self.for_equation()?));
                continue;
            }
            // `(a, b) = f(...)` fills several targets from one call.
            if self.peek() == &Token::LParen {
                if let Some(targets) = self.tuple_targets() {
                    let named = targets
                        .iter()
                        .map(|slot| match slot {
                            Some(Expr::Ref(name)) => Ok(Some(name.clone())),
                            None => Ok(None),
                            Some(other) => Err(self.err(format!(
                                "a target of a tuple inside `when` is a variable, found `{other:?}`"
                            ))),
                        })
                        .collect::<Result<Vec<_>, ParseError>>()?;
                    self.expect(&Token::Assign, "`=` after the tuple of targets")?;
                    let value = self.expr()?;
                    self.opt_string();
                    if self.peek() == &Token::Annotation {
                        self.annotation_body(&mut Annotated::default())?;
                    }
                    self.expect(&Token::Semi, "semicolon after the action")?;
                    actions.push(WhenAction::TupleAssign(named, value));
                    continue;
                }
            }
            let target = self.component_ref()?;
            match (target.as_str(), self.peek()) {
                // `reinit(state, expr)` and `terminate("message")` act on
                // the solver rather than on a variable.
                ("reinit", Token::LParen) => {
                    self.bump();
                    let state = self.component_ref()?;
                    self.expect(&Token::Comma, "comma in reinit")?;
                    let value = self.expr()?;
                    self.expect(&Token::RParen, "closing parenthesis of reinit")?;
                    actions.push(WhenAction::Reinit(state, value));
                }
                ("terminate", Token::LParen) => {
                    self.bump();
                    // The message may be built rather than written out,
                    // the way an `assert` message may: what is kept is
                    // the text of it that stands before the run.
                    let written = self.expr()?;
                    if matches!(written, Expr::Number(_) | Expr::Bool(_)) {
                        return Err(self.err("terminate expects a string message".to_string()));
                    }
                    let message = super::statements::message_text(&written);
                    self.expect(&Token::RParen, "closing parenthesis of terminate")?;
                    actions.push(WhenAction::Terminate(message));
                }
                // Anything else is an equation for a discrete variable.
                (_, _) => {
                    self.expect(&Token::Assign, "`=` in an equation inside when")?;
                    let value = self.expr()?;
                    actions.push(WhenAction::Assign(target, value));
                }
            }
            self.opt_string();
            if self.peek() == &Token::Annotation {
                self.annotation_body(&mut Annotated::default())?;
            }
            self.expect(&Token::Semi, "semicolon after the action")?;
        }
        Ok(actions)
    }

    /// `transition(from, to, condition, immediate = true, reset = true,
    /// synchronize = false, priority = 1);`
    pub(super) fn transition_clause(&mut self) -> Result<Transition, ParseError> {
        self.expect(&Token::LParen, "parenthesis after transition")?;
        let from = self.dotted_name("the state a transition leaves")?;
        self.expect(&Token::Comma, "comma after the state left")?;
        let to = self.dotted_name("the state a transition arrives at")?;
        self.expect(&Token::Comma, "comma before the transition condition")?;
        let condition = self.expr()?;
        let (mut reset, mut priority) = (true, 1);
        let (mut immediate, mut synchronize) = (true, false);
        while self.peek() == &Token::Comma {
            self.bump();
            let name = self.ident("the name of a transition setting")?;
            self.expect(&Token::Assign, "`=` in a transition setting")?;
            let value = self.expr()?;
            let truth = |value: &Expr| matches!(value, Expr::Bool(true) | Expr::Number(1.0));
            match name.as_str() {
                "reset" => reset = truth(&value),
                "priority" => match value {
                    Expr::Number(number) if number.fract() == 0.0 && number >= 1.0 => {
                        priority = number as i64
                    }
                    other => {
                        return Err(self.err(format!(
                        "the priority of a transition is a whole number from 1, found `{other:?}`"
                    )))
                    }
                },
                "immediate" => immediate = truth(&value),
                "synchronize" => synchronize = truth(&value),
                other => {
                    return Err(self.err(format!(
                        "`{other}` is not a transition setting this compiler knows how to honour"
                    )))
                }
            }
        }
        self.expect(&Token::RParen, "closing parenthesis of transition")?;
        if self.peek() == &Token::Annotation {
            self.annotation_body(&mut Annotated::default())?;
        }
        self.expect(&Token::Semi, "semicolon after transition")?;
        Ok(Transition {
            from,
            to,
            condition,
            reset,
            immediate,
            synchronize,
            priority,
        })
    }

    /// `Connections.root(a);`, `Connections.potentialRoot(a, p);`,
    /// `Connections.branch(a, b);`
    pub(super) fn connections_clause(&mut self) -> Result<GraphClause, ParseError> {
        // The caller looked at `Connections`; what follows the dot
        // says which clause this is.
        self.bump();
        self.expect(&Token::Dot, "dot after Connections")?;
        let which = self.ident("the name of a Connections clause")?;
        self.expect(&Token::LParen, "parenthesis after a Connections clause")?;
        let first = self.dotted_name("the node named")?;
        let clause = match which.as_str() {
            "root" => GraphClause::Root(first),
            "potentialRoot" => {
                let mut priority = 0;
                if self.peek() == &Token::Comma {
                    self.bump();
                    match self.bump() {
                        Token::Number(number) if number.fract() == 0.0 => priority = number as i64,
                        other => {
                            return Err(self.err(format!(
                            "the priority of a potential root is a whole number, found `{other}`"
                        )))
                        }
                    }
                }
                GraphClause::PotentialRoot(first, priority)
            }
            "branch" => {
                self.expect(&Token::Comma, "comma between the ends of a branch")?;
                GraphClause::Branch(first, self.dotted_name("the other end")?)
            }
            other => {
                return Err(self.err(format!(
                    "`Connections.{other}` is not a clause this compiler knows"
                )))
            }
        };
        self.expect(
            &Token::RParen,
            "closing parenthesis of a Connections clause",
        )?;
        if self.peek() == &Token::Annotation {
            self.annotation_body(&mut Annotated::default())?;
        }
        self.expect(&Token::Semi, "semicolon after a Connections clause")?;
        Ok(clause)
    }
}

/// One branch of an `if` equation, with the `if` equations written
/// inside it folded into the chain it belongs to.
///
/// An inner chain is spread over the outer branch by conjunction:
/// `if A then e1; elseif B then if C then e2; else e3; end if; end if;`
/// becomes the three branches `A`, `B and C`, `B` - the last standing
/// for `B and not C`, which is what testing them in order means. The
/// equations the branch holds of its own go on every leaf: exactly one
/// of them is chosen, so each such equation is still stated once.
///
/// An inner chain with no `else` covers only part of the branch, so an
/// empty leaf is added to carry the rest of it; without that, a false
/// inner condition would fall through to the branch after the one it
/// was written in.
fn flatten_branch(condition: Option<Expr>, body: BranchBody) -> Vec<IfBranch> {
    let mut out = vec![IfBranch {
        condition,
        equations: body.equations,
        connects: body.connects,
        asserts: body.asserts,
        loops: body.loops,
    }];
    for inner in body.nested {
        let mut leaves = inner.branches;
        if leaves.iter().all(|leaf| leaf.condition.is_some()) {
            leaves.push(IfBranch {
                condition: None,
                equations: Vec::new(),
                connects: Vec::new(),
                asserts: Vec::new(),
                loops: Vec::new(),
            });
        }
        let mut next = Vec::new();
        for outer in &out {
            for leaf in &leaves {
                let condition = match (&outer.condition, &leaf.condition) {
                    (None, inner) => inner.clone(),
                    (outer, None) => outer.clone(),
                    (Some(outer), Some(inner)) => {
                        Some(Expr::And(Box::new(outer.clone()), Box::new(inner.clone())))
                    }
                };
                let mut equations = outer.equations.clone();
                equations.extend(leaf.equations.iter().cloned());
                let mut connects = outer.connects.clone();
                connects.extend(leaf.connects.iter().cloned());
                let mut asserts = outer.asserts.clone();
                asserts.extend(leaf.asserts.iter().cloned());
                let mut loops = outer.loops.clone();
                loops.extend(leaf.loops.iter().cloned());
                next.push(IfBranch {
                    condition,
                    equations,
                    connects,
                    asserts,
                    loops,
                });
            }
        }
        out = next;
    }
    out
}

/// The name of a call, for a message that has to say which one.
fn sketch_call(call: &Expr) -> String {
    match call {
        Expr::Call(name, _) => format!("{name}(...)"),
        other => format!("{other:?}"),
    }
}
