//! The equation section: ordinary equations, `for` and `if` equations,
//! `connect`, `when`, and the clauses that draw a state machine or an
//! overconstrained graph.

use super::*;

impl Parser {
    pub(super) fn equation_item(&mut self) -> Result<EquationItem, ParseError> {
        // `(a, b) = f(...)` fills several targets from one call. Only
        // a top-level comma tells it from a parenthesised expression,
        // so the tuple is tried first and abandoned without a trace.
        let lhs = match self.tuple_targets() {
            Some(targets) => Expr::Tuple(targets),
            None => self.expr()?,
        };
        self.expect(&Token::Assign, "`=` in equation")?;
        let rhs = self.expr()?;
        self.opt_string();
        if self.peek() == &Token::Annotation {
            self.annotation_body(&mut Annotated::default())?;
        }
        self.expect(&Token::Semi, "semicolon after equation")?;
        Ok(EquationItem { lhs, rhs })
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
        loop {
            let condition = self.expr()?;
            self.expect(&Token::Then, "then after the condition of an if equation")?;
            let (equations, connects) = self.branch_body()?;
            branches.push(IfBranch {
                condition: Some(condition),
                equations,
                connects,
            });
            match self.peek() {
                Token::ElseIf => {
                    self.bump();
                }
                Token::Else => {
                    self.bump();
                    let (equations, connects) = self.branch_body()?;
                    branches.push(IfBranch {
                        condition: None,
                        equations,
                        connects,
                    });
                    break;
                }
                _ => break,
            }
        }
        self.expect(&Token::End, "end if")?;
        self.expect(&Token::If, "if after end")?;
        self.expect(&Token::Semi, "semicolon after end if")?;
        if branches
            .iter()
            .all(|b| b.equations.is_empty() && b.connects.is_empty())
        {
            return Err(self.err("if equation has no equations".into()));
        }
        Ok(IfEquation { branches })
    }

    /// Equations and `connect` statements of one branch, up to the next
    /// `elseif`, `else` or `end`.
    pub(super) fn branch_body(&mut self) -> Result<BranchBody, ParseError> {
        let mut equations = Vec::new();
        let mut connects = Vec::new();
        loop {
            match self.peek() {
                Token::ElseIf | Token::Else | Token::End => break,
                Token::Eof => return Err(self.err("unterminated if equation".into())),
                Token::Connect => connects.push(self.connect_clause()?),
                _ => equations.push(self.equation_item()?),
            }
        }
        Ok((equations, connects))
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
                    let message = match self.bump() {
                        Token::Str(message) => message,
                        other => {
                            return Err(self.err(format!(
                                "terminate expects a string message, found `{other}`"
                            )))
                        }
                    };
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
        let (mut reset, mut priority, mut immediate) = (true, 1, true);
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
                // An arrow that waits for the other machines to finish
                // needs machines to wait for, which is a state holding
                // one - and a state holds equations here, not a machine.
                "synchronize" if !truth(&value) => {}
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
