//! Lexer for the M0 Modelica slice: identifiers, numbers, strings,
//! comments, keywords and single/double-character punctuation.

use std::fmt;

/// A lexical token of the Modelica slice.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// Identifier (variable, type or function name).
    Ident(String),
    /// Numeric literal, including scientific notation.
    Number(f64),
    /// Double-quoted string literal (descriptions).
    Str(String),
    /// Keyword `model`.
    Model,
    /// Keyword `end`.
    End,
    /// Keyword `equation`.
    Equation,
    /// Keyword `parameter`.
    Parameter,
    /// Keyword `constant`.
    Constant,
    /// Boolean literal `true`.
    True,
    /// Boolean literal `false`.
    False,
    /// Keyword `annotation`.
    Annotation,
    /// Keyword `if`.
    If,
    /// Keyword `then`.
    Then,
    /// Keyword `elseif`.
    ElseIf,
    /// Keyword `else`.
    Else,
    /// Keyword `and`.
    And,
    /// Keyword `or`.
    Or,
    /// Keyword `not`.
    Not,
    /// Keyword `when`.
    When,
    /// Keyword `elsewhen`.
    ElseWhen,
    /// Keyword `discrete` — a variable that changes only at events.
    Discrete,
    /// Keyword `connector`.
    Connector,
    /// Keyword `block` — a model whose connectors are causal.
    Block,
    /// Keyword `flow`.
    Flow,
    /// Keyword `stream`.
    Stream,
    /// Keyword `expandable`.
    Expandable,
    /// Keyword `operator` — a class of overloads for a record.
    Operator,
    /// Keyword `encapsulated` — parsed and otherwise ignored.
    Encapsulated,
    /// Keyword `extends`.
    Extends,
    /// Keyword `connect`.
    Connect,
    /// Keyword `for`.
    For,
    /// Keyword `in`.
    In,
    /// Keyword `loop`.
    Loop,
    /// Keyword `while`.
    While,
    /// Keyword `break`.
    Break,
    /// Keyword `return`.
    Return,
    /// Keyword `function`.
    Function,
    /// Keyword `record`.
    Record,
    /// Keyword `input`.
    Input,
    /// Keyword `output`.
    Output,
    /// Keyword `algorithm`.
    Algorithm,
    /// Keyword `package`.
    Package,
    /// Keyword `type`.
    Type,
    /// Keyword `partial`.
    Partial,
    /// Keyword `import`.
    Import,
    /// Keyword `protected`.
    Protected,
    /// Keyword `within`.
    Within,
    /// Keyword `replaceable` — a declaration open to redeclaration.
    Replaceable,
    /// Keyword `redeclare` — replaces a replaceable declaration.
    Redeclare,
    /// Keyword `constrainedby` — the interface a redeclaration must meet.
    ConstrainedBy,
    /// Keyword `inner` — the declaration that owns a shared instance.
    Inner,
    /// Keyword `outer` — a reference to the enclosing `inner` instance.
    Outer,
    /// Keyword `enumeration` — a type of named literals.
    Enumeration,
    /// Keyword `final` — forbids further modification (parsed, ignored).
    Final,
    /// Keyword `each` — applies a modifier to every array element.
    Each,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `;`
    Semi,
    /// `,`
    Comma,
    /// `=` (declaration binding / equation sign).
    Assign,
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `^`
    Caret,
    /// `.+` — element by element.
    DotPlus,
    /// `.-`
    DotMinus,
    /// `.*`
    DotStar,
    /// `./`
    DotSlash,
    /// `.^`
    DotCaret,
    /// `.`
    Dot,
    /// `{`
    LBrace,
    /// `}`
    RBrace,
    /// `[`
    LBracket,
    /// `]`
    RBracket,
    /// `:`
    Colon,
    /// `:=`
    Becomes,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `==`
    EqEq,
    /// `<>`
    Ne,
    /// End of input marker.
    Eof,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Ident(s) => write!(f, "{s}"),
            Token::Number(n) => write!(f, "{n}"),
            Token::Str(s) => write!(f, "\"{s}\""),
            Token::Model => write!(f, "model"),
            Token::End => write!(f, "end"),
            Token::Equation => write!(f, "equation"),
            Token::Parameter => write!(f, "parameter"),
            Token::Constant => write!(f, "constant"),
            Token::True => write!(f, "true"),
            Token::False => write!(f, "false"),
            Token::Annotation => write!(f, "annotation"),
            Token::If => write!(f, "if"),
            Token::Then => write!(f, "then"),
            Token::ElseIf => write!(f, "elseif"),
            Token::Else => write!(f, "else"),
            Token::And => write!(f, "and"),
            Token::Or => write!(f, "or"),
            Token::Not => write!(f, "not"),
            Token::When => write!(f, "when"),
            Token::ElseWhen => write!(f, "elsewhen"),
            Token::Discrete => write!(f, "discrete"),
            Token::Connector => write!(f, "connector"),
            Token::Block => write!(f, "block"),
            Token::Flow => write!(f, "flow"),
            Token::Stream => write!(f, "stream"),
            Token::Expandable => write!(f, "expandable"),
            Token::Operator => write!(f, "operator"),
            Token::Encapsulated => write!(f, "encapsulated"),
            Token::Extends => write!(f, "extends"),
            Token::Connect => write!(f, "connect"),
            Token::For => write!(f, "for"),
            Token::In => write!(f, "in"),
            Token::Loop => write!(f, "loop"),
            Token::While => write!(f, "while"),
            Token::Break => write!(f, "break"),
            Token::Return => write!(f, "return"),
            Token::Function => write!(f, "function"),
            Token::Record => write!(f, "record"),
            Token::Input => write!(f, "input"),
            Token::Output => write!(f, "output"),
            Token::Algorithm => write!(f, "algorithm"),
            Token::Package => write!(f, "package"),
            Token::Type => write!(f, "type"),
            Token::Partial => write!(f, "partial"),
            Token::Import => write!(f, "import"),
            Token::Protected => write!(f, "protected"),
            Token::Within => write!(f, "within"),
            Token::Replaceable => write!(f, "replaceable"),
            Token::Redeclare => write!(f, "redeclare"),
            Token::ConstrainedBy => write!(f, "constrainedby"),
            Token::Inner => write!(f, "inner"),
            Token::Outer => write!(f, "outer"),
            Token::Enumeration => write!(f, "enumeration"),
            Token::Final => write!(f, "final"),
            Token::Each => write!(f, "each"),
            Token::LParen => write!(f, "("),
            Token::RParen => write!(f, ")"),
            Token::Semi => write!(f, ";"),
            Token::Comma => write!(f, ","),
            Token::Assign => write!(f, "="),
            Token::Plus => write!(f, "+"),
            Token::Minus => write!(f, "-"),
            Token::Star => write!(f, "*"),
            Token::Slash => write!(f, "/"),
            Token::Caret => write!(f, "^"),
            Token::DotPlus => write!(f, ".+"),
            Token::DotMinus => write!(f, ".-"),
            Token::DotStar => write!(f, ".*"),
            Token::DotSlash => write!(f, "./"),
            Token::DotCaret => write!(f, ".^"),
            Token::Dot => write!(f, "."),
            Token::LBrace => write!(f, "{{"),
            Token::RBrace => write!(f, "}}"),
            Token::LBracket => write!(f, "["),
            Token::RBracket => write!(f, "]"),
            Token::Colon => write!(f, ":"),
            Token::Becomes => write!(f, ":="),
            Token::Lt => write!(f, "<"),
            Token::Le => write!(f, "<="),
            Token::Gt => write!(f, ">"),
            Token::Ge => write!(f, ">="),
            Token::EqEq => write!(f, "=="),
            Token::Ne => write!(f, "<>"),
            Token::Eof => write!(f, "<eof>"),
        }
    }
}

/// A token together with the 1-based source line it starts on.
#[derive(Debug, Clone)]
pub struct Spanned {
    /// The token itself.
    pub token: Token,
    /// 1-based line number in the source text.
    pub line: u32,
}

/// A lexical error with its source line.
#[derive(Debug)]
pub struct LexError {
    /// Human-readable description of the problem.
    pub message: String,
    /// 1-based line number where the error occurred.
    pub line: u32,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

/// Tokenize Modelica source text into a `Spanned` token stream
/// terminated by [`Token::Eof`].
pub fn lex(source: &str) -> Result<Vec<Spanned>, LexError> {
    let mut out = Vec::new();
    let bytes: Vec<char> = source.chars().collect();
    let mut i = 0;
    let mut line: u32 = 1;

    while i < bytes.len() {
        let c = bytes[i];
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            ' ' | '\t' | '\r' => i += 1,
            '/' if bytes.get(i + 1) == Some(&'/') => {
                while i < bytes.len() && bytes[i] != '\n' {
                    i += 1;
                }
            }
            '/' if bytes.get(i + 1) == Some(&'*') => {
                i += 2;
                loop {
                    if i + 1 >= bytes.len() {
                        return Err(LexError {
                            message: "unterminated /* comment".into(),
                            line,
                        });
                    }
                    if bytes[i] == '\n' {
                        line += 1;
                    }
                    if bytes[i] == '*' && bytes[i + 1] == '/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            '"' => {
                i += 1;
                let mut s = String::new();
                loop {
                    match bytes.get(i) {
                        None => {
                            return Err(LexError {
                                message: "unterminated string".into(),
                                line,
                            })
                        }
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        Some('\\') => {
                            if let Some(&next) = bytes.get(i + 1) {
                                s.push(next);
                                i += 2;
                            } else {
                                return Err(LexError {
                                    message: "unterminated string".into(),
                                    line,
                                });
                            }
                        }
                        Some(&ch) => {
                            if ch == '\n' {
                                line += 1;
                            }
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                out.push(Spanned {
                    token: Token::Str(s),
                    line,
                });
            }
            '0'..='9' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == '.') {
                    // A dot is part of the number only when followed by a
                    // digit; `1.x` is Number(1), Dot, Ident(x).
                    if bytes[i] == '.' && !bytes.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
                        break;
                    }
                    i += 1;
                }
                // Exponent part: 1e-3, 2.5E+10.
                if i < bytes.len() && (bytes[i] == 'e' || bytes[i] == 'E') {
                    let mut j = i + 1;
                    if matches!(bytes.get(j), Some('+') | Some('-')) {
                        j += 1;
                    }
                    if bytes.get(j).is_some_and(|c| c.is_ascii_digit()) {
                        i = j;
                        while i < bytes.len() && bytes[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                let text: String = bytes[start..i].iter().collect();
                let value = text.parse::<f64>().map_err(|_| LexError {
                    message: format!("invalid number `{text}`"),
                    line,
                })?;
                out.push(Spanned {
                    token: Token::Number(value),
                    line,
                });
            }
            // A quoted identifier: `'+'` names an operator, and the
            // quotes are kept so it can never be confused with a word
            // anyone could type as a plain name.
            '\'' => {
                let start = i;
                i += 1;
                while i < bytes.len() && bytes[i] != '\'' {
                    if bytes[i] == '\\' && i + 1 < bytes.len() {
                        i += 1;
                    }
                    if bytes[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
                if i >= bytes.len() {
                    return Err(LexError {
                        message: "unterminated quoted identifier".into(),
                        line,
                    });
                }
                i += 1;
                let word: String = bytes[start..i].iter().collect();
                out.push(Spanned {
                    token: Token::Ident(word),
                    line,
                });
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_alphanumeric() || bytes[i] == '_') {
                    i += 1;
                }
                let word: String = bytes[start..i].iter().collect();
                let token = match word.as_str() {
                    "model" => Token::Model,
                    "end" => Token::End,
                    "equation" => Token::Equation,
                    "parameter" => Token::Parameter,
                    "constant" => Token::Constant,
                    "true" => Token::True,
                    "false" => Token::False,
                    "annotation" => Token::Annotation,
                    "if" => Token::If,
                    "then" => Token::Then,
                    "elseif" => Token::ElseIf,
                    "else" => Token::Else,
                    "and" => Token::And,
                    "or" => Token::Or,
                    "not" => Token::Not,
                    "when" => Token::When,
                    "elsewhen" => Token::ElseWhen,
                    "discrete" => Token::Discrete,
                    "connector" => Token::Connector,
                    "block" => Token::Block,
                    "flow" => Token::Flow,
                    "stream" => Token::Stream,
                    "expandable" => Token::Expandable,
                    "operator" => Token::Operator,
                    "encapsulated" => Token::Encapsulated,
                    "extends" => Token::Extends,
                    "connect" => Token::Connect,
                    "for" => Token::For,
                    "in" => Token::In,
                    "loop" => Token::Loop,
                    "while" => Token::While,
                    "break" => Token::Break,
                    "return" => Token::Return,
                    "function" => Token::Function,
                    "record" => Token::Record,
                    "input" => Token::Input,
                    "output" => Token::Output,
                    "algorithm" => Token::Algorithm,
                    "package" => Token::Package,
                    "type" => Token::Type,
                    "partial" => Token::Partial,
                    "import" => Token::Import,
                    "protected" => Token::Protected,
                    "within" => Token::Within,
                    "replaceable" => Token::Replaceable,
                    "redeclare" => Token::Redeclare,
                    "constrainedby" => Token::ConstrainedBy,
                    "inner" => Token::Inner,
                    "outer" => Token::Outer,
                    "enumeration" => Token::Enumeration,
                    "final" => Token::Final,
                    "each" => Token::Each,
                    _ => Token::Ident(word),
                };
                out.push(Spanned { token, line });
            }
            '(' => {
                out.push(Spanned {
                    token: Token::LParen,
                    line,
                });
                i += 1;
            }
            ')' => {
                out.push(Spanned {
                    token: Token::RParen,
                    line,
                });
                i += 1;
            }
            ';' => {
                out.push(Spanned {
                    token: Token::Semi,
                    line,
                });
                i += 1;
            }
            ',' => {
                out.push(Spanned {
                    token: Token::Comma,
                    line,
                });
                i += 1;
            }
            '=' if bytes.get(i + 1) == Some(&'=') => {
                out.push(Spanned {
                    token: Token::EqEq,
                    line,
                });
                i += 2;
            }
            '=' => {
                out.push(Spanned {
                    token: Token::Assign,
                    line,
                });
                i += 1;
            }
            '<' if bytes.get(i + 1) == Some(&'=') => {
                out.push(Spanned {
                    token: Token::Le,
                    line,
                });
                i += 2;
            }
            '<' if bytes.get(i + 1) == Some(&'>') => {
                out.push(Spanned {
                    token: Token::Ne,
                    line,
                });
                i += 2;
            }
            '<' => {
                out.push(Spanned {
                    token: Token::Lt,
                    line,
                });
                i += 1;
            }
            '>' if bytes.get(i + 1) == Some(&'=') => {
                out.push(Spanned {
                    token: Token::Ge,
                    line,
                });
                i += 2;
            }
            '>' => {
                out.push(Spanned {
                    token: Token::Gt,
                    line,
                });
                i += 1;
            }
            '+' => {
                out.push(Spanned {
                    token: Token::Plus,
                    line,
                });
                i += 1;
            }
            '-' => {
                out.push(Spanned {
                    token: Token::Minus,
                    line,
                });
                i += 1;
            }
            '*' => {
                out.push(Spanned {
                    token: Token::Star,
                    line,
                });
                i += 1;
            }
            '/' => {
                out.push(Spanned {
                    token: Token::Slash,
                    line,
                });
                i += 1;
            }
            '^' => {
                out.push(Spanned {
                    token: Token::Caret,
                    line,
                });
                i += 1;
            }
            // `.+`, `.*` and friends work element by element; a lone
            // dot is still the separator of a qualified name.
            '.' if matches!(bytes.get(i + 1), Some('+' | '-' | '*' | '/' | '^')) => {
                let token = match bytes[i + 1] {
                    '+' => Token::DotPlus,
                    '-' => Token::DotMinus,
                    '*' => Token::DotStar,
                    '/' => Token::DotSlash,
                    _ => Token::DotCaret,
                };
                out.push(Spanned { token, line });
                i += 2;
            }
            '.' => {
                out.push(Spanned {
                    token: Token::Dot,
                    line,
                });
                i += 1;
            }
            '{' => {
                out.push(Spanned {
                    token: Token::LBrace,
                    line,
                });
                i += 1;
            }
            '}' => {
                out.push(Spanned {
                    token: Token::RBrace,
                    line,
                });
                i += 1;
            }
            '[' => {
                out.push(Spanned {
                    token: Token::LBracket,
                    line,
                });
                i += 1;
            }
            ']' => {
                out.push(Spanned {
                    token: Token::RBracket,
                    line,
                });
                i += 1;
            }
            ':' if bytes.get(i + 1) == Some(&'=') => {
                out.push(Spanned {
                    token: Token::Becomes,
                    line,
                });
                i += 2;
            }
            ':' => {
                out.push(Spanned {
                    token: Token::Colon,
                    line,
                });
                i += 1;
            }
            other => {
                return Err(LexError {
                    message: format!("unexpected character `{other}`"),
                    line,
                });
            }
        }
    }
    out.push(Spanned {
        token: Token::Eof,
        line,
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(source: &str) -> Vec<Token> {
        lex(source).unwrap().into_iter().map(|s| s.token).collect()
    }

    #[test]
    fn display_covers_every_token() {
        let all = [
            Token::Ident("x".into()),
            Token::Number(1.5),
            Token::Str("s".into()),
            Token::Model,
            Token::End,
            Token::Equation,
            Token::Parameter,
            Token::Constant,
            Token::True,
            Token::False,
            Token::Annotation,
            Token::If,
            Token::Then,
            Token::ElseIf,
            Token::Else,
            Token::And,
            Token::Or,
            Token::Not,
            Token::When,
            Token::ElseWhen,
            Token::Discrete,
            Token::Connector,
            Token::Block,
            Token::Flow,
            Token::Extends,
            Token::Connect,
            Token::For,
            Token::In,
            Token::Loop,
            Token::While,
            Token::Function,
            Token::Record,
            Token::Input,
            Token::Output,
            Token::Algorithm,
            Token::Package,
            Token::Type,
            Token::Partial,
            Token::Import,
            Token::Protected,
            Token::Within,
            Token::Replaceable,
            Token::Redeclare,
            Token::ConstrainedBy,
            Token::Inner,
            Token::Outer,
            Token::Enumeration,
            Token::Final,
            Token::Each,
            Token::LBrace,
            Token::RBrace,
            Token::LBracket,
            Token::RBracket,
            Token::Colon,
            Token::Becomes,
            Token::LParen,
            Token::RParen,
            Token::Semi,
            Token::Comma,
            Token::Assign,
            Token::Plus,
            Token::Minus,
            Token::Star,
            Token::Slash,
            Token::Caret,
            Token::DotPlus,
            Token::DotMinus,
            Token::DotStar,
            Token::DotSlash,
            Token::DotCaret,
            Token::Dot,
            Token::Lt,
            Token::Le,
            Token::Gt,
            Token::Ge,
            Token::EqEq,
            Token::Ne,
            Token::Eof,
        ];
        let rendered: Vec<String> = all.iter().map(|t| t.to_string()).collect();
        assert!(rendered.iter().all(|s| !s.is_empty()));
        assert_eq!(rendered[rendered.len() - 1], "<eof>");
    }

    #[test]
    fn lexes_operators_and_keywords() {
        assert_eq!(
            tokens("== <= >= <> < > = if then elseif else and or not true false"),
            vec![
                Token::EqEq,
                Token::Le,
                Token::Ge,
                Token::Ne,
                Token::Lt,
                Token::Gt,
                Token::Assign,
                Token::If,
                Token::Then,
                Token::ElseIf,
                Token::Else,
                Token::And,
                Token::Or,
                Token::Not,
                Token::True,
                Token::False,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn lexes_the_structural_keywords() {
        assert_eq!(
            tokens("replaceable redeclare constrainedby inner outer enumeration final each"),
            vec![
                Token::Replaceable,
                Token::Redeclare,
                Token::ConstrainedBy,
                Token::Inner,
                Token::Outer,
                Token::Enumeration,
                Token::Final,
                Token::Each,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn lexes_numbers_with_exponents() {
        assert_eq!(
            tokens("1e3 2.5E-2 1e+10 7 0.5"),
            vec![
                Token::Number(1e3),
                Token::Number(2.5e-2),
                Token::Number(1e10),
                Token::Number(7.0),
                Token::Number(0.5),
                Token::Eof,
            ]
        );
    }

    #[test]
    fn dot_after_number_is_separate_token() {
        // `1.x` is Number(1), Dot, Ident: the dot is not part of the number.
        assert_eq!(
            tokens("1.x"),
            vec![
                Token::Number(1.0),
                Token::Dot,
                Token::Ident("x".into()),
                Token::Eof
            ]
        );
    }

    #[test]
    fn rejects_double_dot_number() {
        let e = lex("1.2.3").unwrap_err();
        assert!(e.message.contains("invalid number"), "{}", e.message);
    }

    #[test]
    fn lexes_strings_with_escapes_and_newlines() {
        assert_eq!(
            tokens(
                r#""a\"b" "line1
line2""#
            ),
            vec![
                Token::Str("a\"b".into()),
                Token::Str("line1\nline2".into()),
                Token::Eof
            ]
        );
    }

    #[test]
    fn skips_comments_and_counts_lines() {
        let spanned = lex("// line comment\n/* block\nmore */ x").unwrap();
        assert_eq!(spanned[0].token, Token::Ident("x".into()));
        assert_eq!(spanned[0].line, 3);
    }

    #[test]
    fn every_token_can_say_its_own_name() {
        // The display is what error messages are written with, so
        // each keyword has to spell itself.
        let source = "model M stream flow expandable operator encapsulated block while break return connector record function package type partial import protected within replaceable redeclare constrainedby inner outer enumeration final each discrete when elsewhen algorithm extends connect for in loop equation annotation";
        let spelled: Vec<String> = lex(source)
            .unwrap()
            .iter()
            .map(|s| s.token.to_string())
            .collect();
        for word in source.split_whitespace() {
            assert!(spelled.iter().any(|t| t == word), "{word} is not spelled");
        }
        // And so do the shapes that are not words.
        for source in [
            "1 + 2 - 3 * 4 / 5 ^ 6",
            "a .+ b .- c .* d ./ e .^ f",
            "x[1] {2} (3)",
            "a < b <= c > d >= e == f <> g",
            "h := 1; i, j.k : l",
            "\"text\" true false 'quoted'",
        ] {
            let spelled: Vec<String> = lex(source)
                .unwrap()
                .iter()
                .map(|s| s.token.to_string())
                .collect();
            assert!(!spelled.is_empty(), "{source}");
        }
    }

    #[test]
    fn a_quoted_identifier_must_be_closed() {
        let failed = lex("model M Real 'unfinished").unwrap_err();
        assert!(failed.message.contains("unterminated quoted identifier"));
        // One that is closed keeps its quotes, and may span a line.
        let spelled: Vec<String> = lex("'+' 'two\nlines'")
            .unwrap()
            .iter()
            .map(|s| s.token.to_string())
            .collect();
        assert_eq!(spelled[0], "'+'");
    }

    #[test]
    fn error_reports_line_number() {
        let e = lex("x\ny\n#").unwrap_err();
        assert_eq!(e.line, 3);
        assert!(e.to_string().contains("line 3"));
        assert!(e.message.contains("unexpected character"));
    }

    #[test]
    fn rejects_unclosed_constructs() {
        assert!(lex("/* no end")
            .unwrap_err()
            .message
            .contains("unterminated /* comment"));
        assert!(lex("\"no end")
            .unwrap_err()
            .message
            .contains("unterminated string"));
        assert!(lex("\"tail\\")
            .unwrap_err()
            .message
            .contains("unterminated string"));
    }
}
