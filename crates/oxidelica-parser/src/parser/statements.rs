//! Algorithm sections: assignments and the control flow around them.

use super::*;

impl Parser {
    /// `(condition, "message")` after the word `assert`, wherever it
    /// was written. An optional third argument names the assertion
    /// level; it is accepted and not distinguished.
    pub(super) fn assert_arguments(&mut self) -> Result<(Expr, String), ParseError> {
        self.expect(&Token::LParen, "parenthesis after assert")?;
        let condition = self.expr()?;
        self.expect(&Token::Comma, "comma before the assert message")?;
        let message = match self.bump() {
            Token::Str(message) => message,
            other => {
                return Err(self.err(format!("assert expects a string message, found `{other}`")))
            }
        };
        if self.peek() == &Token::Comma {
            self.bump();
            self.dotted_name("assertion level")?;
        }
        self.expect(&Token::RParen, "closing parenthesis of assert")?;
        Ok((condition, message))
    }

    /// An algorithm section: assignments, `if` and `for` statements, up
    /// to whatever ends the section.
    pub(super) fn statements(&mut self) -> Result<Vec<Statement>, ParseError> {
        let mut out = Vec::new();
        loop {
            match self.peek() {
                // A name followed by a parenthesis is a call standing on
                // its own, not something being assigned to.
                Token::Ident(name) if self.peek_at(1) == &Token::LParen => {
                    let called = name.clone();
                    self.bump();
                    match called.as_str() {
                        // Inside a loop this is one check per round,
                        // with the loop variable already folded in.
                        "assert" => {
                            let (condition, message) = self.assert_arguments()?;
                            self.expect(&Token::Semi, "semicolon after assert")?;
                            out.push(Statement::Assert(condition, message));
                        }
                        "terminate" => {
                            return Err(self.err(
                                "`terminate` among the statements would end the run the moment \
                                 the section is reached; it belongs in a `when`"
                                    .to_string(),
                            ))
                        }
                        // Every function here is pure, so a call whose
                        // outputs go nowhere cannot do anything, and
                        // writing one is a mistake rather than an intent.
                        _ => {
                            return Err(self.err(format!(
                                "`{called}(...)` on its own does nothing: a function here has \
                                 no way to act but through its outputs, so its answer has to \
                                 go somewhere"
                            )))
                        }
                    }
                }
                Token::Ident(_) => {
                    let target = self.component_ref()?;
                    // `c[i] := ...` assigns one element of an array.
                    let mut subscripts = Vec::new();
                    if self.peek() == &Token::LBracket {
                        self.bump();
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
                    }
                    self.expect(&Token::Becomes, "`:=` in assignment")?;
                    let value = self.expr()?;
                    self.opt_string();
                    if self.peek() == &Token::Annotation {
                        self.annotation_body(&mut Experiment::default())?;
                    }
                    self.expect(&Token::Semi, "semicolon after assignment")?;
                    out.push(Statement::Assign(target, subscripts, value));
                }
                // `(a, , c) := f(...);` — nothing else in an algorithm
                // starts with a parenthesis.
                Token::LParen => {
                    self.bump();
                    let mut targets = Vec::new();
                    loop {
                        match self.peek() {
                            Token::Comma | Token::RParen => targets.push(None),
                            _ => targets.push(Some(self.ident("target of a tuple assignment")?)),
                        }
                        match self.bump() {
                            Token::Comma => continue,
                            Token::RParen => break,
                            other => {
                                return Err(self.err(format!(
                                    "expected `,` or `)` in a tuple of targets, found `{other}`"
                                )))
                            }
                        }
                    }
                    self.expect(&Token::Becomes, "`:=` after the tuple of targets")?;
                    let value = self.expr()?;
                    self.opt_string();
                    if self.peek() == &Token::Annotation {
                        self.annotation_body(&mut Experiment::default())?;
                    }
                    self.expect(&Token::Semi, "semicolon after assignment")?;
                    out.push(Statement::TupleAssign(targets, value));
                }
                Token::If => out.push(self.if_statement()?),
                Token::For => out.push(self.for_statement()?),
                Token::When => out.push(self.when_statement()?),
                Token::While => {
                    self.bump();
                    let condition = self.expr()?;
                    self.expect(&Token::Loop, "loop after the condition of a while")?;
                    let body = self.statements()?;
                    self.expect(&Token::End, "end closing the while")?;
                    self.expect(&Token::While, "while after end")?;
                    self.expect(&Token::Semi, "semicolon after end while")?;
                    out.push(Statement::While(condition, body));
                }
                Token::Break => {
                    self.bump();
                    self.expect(&Token::Semi, "semicolon after break")?;
                    out.push(Statement::Break);
                }
                Token::Return => {
                    self.bump();
                    self.expect(&Token::Semi, "semicolon after return")?;
                    out.push(Statement::Return);
                }
                _ => break,
            }
        }
        Ok(out)
    }

    /// `when c then … elsewhen … end when;` inside an algorithm.
    pub(super) fn when_statement(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::When, "when")?;
        let mut branches = Vec::new();
        loop {
            let condition = self.expr()?;
            self.expect(&Token::Then, "then after a when condition")?;
            let body = self.statements()?;
            // A vector condition is a branch apiece here too, and for
            // the same reason.
            match condition {
                Expr::Array(conditions) if !conditions.is_empty() => {
                    for condition in conditions {
                        branches.push(StatementBranch {
                            condition: Some(condition),
                            body: body.clone(),
                        });
                    }
                }
                condition => branches.push(StatementBranch {
                    condition: Some(condition),
                    body,
                }),
            }
            if self.peek() == &Token::ElseWhen {
                self.bump();
                continue;
            }
            break;
        }
        self.expect(&Token::End, "end closing the when")?;
        self.expect(&Token::When, "when after end")?;
        self.expect(&Token::Semi, "semicolon after end when")?;
        Ok(Statement::When(branches))
    }

    /// `if c then … elseif … else … end if;` inside an algorithm.
    pub(super) fn if_statement(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::If, "if")?;
        let mut branches = Vec::new();
        loop {
            let condition = self.expr()?;
            self.expect(&Token::Then, "then after the condition of an if statement")?;
            branches.push(StatementBranch {
                condition: Some(condition),
                body: self.statements()?,
            });
            match self.peek() {
                Token::ElseIf => {
                    self.bump();
                }
                Token::Else => {
                    self.bump();
                    branches.push(StatementBranch {
                        condition: None,
                        body: self.statements()?,
                    });
                    break;
                }
                _ => break,
            }
        }
        self.expect(&Token::End, "end if")?;
        self.expect(&Token::If, "if after end")?;
        self.expect(&Token::Semi, "semicolon after end if")?;
        Ok(Statement::If(branches))
    }

    /// `for <indices> loop … end for;` inside an algorithm. Several
    /// indices nest, the same way they do among equations.
    pub(super) fn for_statement(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::For, "for")?;
        let indices = self.for_indices()?;
        self.expect(&Token::Loop, "loop after the range")?;
        let mut body = self.statements()?;
        self.expect(&Token::End, "end for")?;
        self.expect(&Token::For, "for after end")?;
        self.expect(&Token::Semi, "semicolon after end for")?;
        if body.is_empty() {
            return Err(self.err("for statement has no body".into()));
        }
        let mut built: Option<Statement> = None;
        for (variable, range) in indices.into_iter().rev() {
            built = Some(Statement::For(
                variable,
                range,
                match built {
                    Some(inner) => vec![inner],
                    None => std::mem::take(&mut body),
                },
            ));
        }
        Ok(built.expect("a loop has at least one index"))
    }
}
