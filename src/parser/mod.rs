//! # Parser
//!
//! Converts a shell input string into an [`Expr`] AST.
//!
//! This is a hand-written recursive-descent parser that supports a useful
//! subset of Bash syntax without depending on any external parser crate.

pub mod ast;
pub mod lexer;

pub use ast::{Expr, RedirectMode, Word};
pub use lexer::{Lexer, Token};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("unexpected token: {0:?}")]
    UnexpectedToken(Token),
    #[error("unexpected end of input")]
    UnexpectedEof,
    #[error("unmatched '{0}'")]
    Unmatched(char),
    #[error("empty command")]
    EmptyCommand,
}

pub type ParseResult<T> = Result<T, ParseError>;

/// Parse a shell input string into an expression tree.
pub fn parse(input: &str) -> ParseResult<Expr> {
    let tokens = Lexer::new(input).tokenize()?;
    let mut parser = Parser::new(tokens);
    let expr = parser.parse_list()?;
    if !parser.is_done() {
        return Err(ParseError::UnexpectedToken(parser.peek().clone()));
    }
    Ok(expr)
}

// ─── Parser ──────────────────────────────────────────────────────────────────

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> &Token {
        let tok = self.tokens.get(self.pos).unwrap_or(&Token::Eof);
        self.pos += 1;
        tok
    }

    fn is_done(&self) -> bool {
        matches!(self.peek(), Token::Eof)
    }

    fn expect_word(&mut self) -> ParseResult<Word> {
        match self.advance().clone() {
            Token::Word(parts) => Ok(parts),
            Token::Eof => Err(ParseError::UnexpectedEof),
            other => Err(ParseError::UnexpectedToken(other)),
        }
    }

    /// list = pipeline ( ('&&'|'||'|';'|'&') pipeline )*
    fn parse_list(&mut self) -> ParseResult<Expr> {
        let mut left = self.parse_pipeline()?;

        loop {
            match self.peek().clone() {
                Token::And => {
                    self.advance();
                    let right = self.parse_pipeline()?;
                    left = Expr::And {
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                Token::Or => {
                    self.advance();
                    let right = self.parse_pipeline()?;
                    left = Expr::Or {
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                Token::Semi => {
                    self.advance();
                    if self.is_done()
                        || matches!(
                            self.peek(),
                            Token::RParen | Token::And | Token::Or | Token::Semi
                        )
                    {
                        // trailing semicolon — wrap as sequence with no RHS
                        // just return left as-is (bash behaviour)
                        break;
                    }
                    let right = self.parse_pipeline()?;
                    left = Expr::Sequence {
                        left: Box::new(left),
                        right: Box::new(right),
                    };
                }
                _ => break,
            }
        }

        Ok(left)
    }

    /// pipeline = command ('|' command)*
    fn parse_pipeline(&mut self) -> ParseResult<Expr> {
        let mut left = self.parse_redirect()?;

        while matches!(self.peek(), Token::Pipe) {
            self.advance();
            let right = self.parse_redirect()?;
            left = Expr::Pipe {
                left: Box::new(left),
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    /// redirect = command (redirection)*
    fn parse_redirect(&mut self) -> ParseResult<Expr> {
        let mut cmd = self.parse_command()?;

        loop {
            match self.peek().clone() {
                Token::RedirectOut => {
                    self.advance();
                    let file = self.expect_word()?;
                    cmd = Expr::Redirect {
                        expr: Box::new(cmd),
                        file,
                        mode: RedirectMode::Overwrite,
                    };
                }
                Token::RedirectAppend => {
                    self.advance();
                    let file = self.expect_word()?;
                    cmd = Expr::Redirect {
                        expr: Box::new(cmd),
                        file,
                        mode: RedirectMode::Append,
                    };
                }
                Token::RedirectIn => {
                    self.advance();
                    let file = self.expect_word()?;
                    cmd = Expr::Redirect {
                        expr: Box::new(cmd),
                        file,
                        mode: RedirectMode::Input,
                    };
                }
                _ => break,
            }
        }

        Ok(cmd)
    }

    /// command = subshell | simple_command
    fn parse_command(&mut self) -> ParseResult<Expr> {
        if matches!(self.peek(), Token::LParen) {
            return self.parse_subshell();
        }
        self.parse_simple_command()
    }

    fn parse_subshell(&mut self) -> ParseResult<Expr> {
        // consume '('
        self.advance();
        let inner = self.parse_list()?;
        match self.advance().clone() {
            Token::RParen => {}
            Token::Eof => return Err(ParseError::Unmatched('(')),
            other => return Err(ParseError::UnexpectedToken(other)),
        }
        Ok(Expr::Subshell {
            expr: Box::new(inner),
        })
    }

    fn parse_simple_command(&mut self) -> ParseResult<Expr> {
        let mut words: Vec<Word> = Vec::new();

        loop {
            match self.peek().clone() {
                Token::Word(w) => {
                    self.advance();
                    words.push(w);
                }
                // Stop collecting words at these tokens
                Token::Pipe
                | Token::And
                | Token::Or
                | Token::Semi
                | Token::RParen
                | Token::RedirectOut
                | Token::RedirectAppend
                | Token::RedirectIn
                | Token::Eof => break,
                other => return Err(ParseError::UnexpectedToken(other)),
            }
        }

        if words.is_empty() {
            return Err(ParseError::EmptyCommand);
        }

        let name = words.remove(0);
        Ok(Expr::Command { name, args: words })
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::ast::WordPart;

    use super::*;

    fn cmd(name: &str, args: &[&str]) -> Expr {
        Expr::Command {
            name: Word(vec![WordPart::Literal(name.to_string())]),
            args: args
                .iter()
                .map(|a| Word(vec![WordPart::Literal(a.to_string())]))
                .collect(),
        }
    }

    #[test]
    fn test_simple_command() {
        let expr = parse("echo hello").unwrap();
        assert_eq!(expr, cmd("echo", &["hello"]));
    }

    #[test]
    fn test_command_multiple_args() {
        let expr = parse("ls -la /tmp").unwrap();
        assert_eq!(expr, cmd("ls", &["-la", "/tmp"]));
    }

    #[test]
    fn test_pipe() {
        let expr = parse("cat file.txt | grep foo").unwrap();
        assert_eq!(
            expr,
            Expr::Pipe {
                left: Box::new(cmd("cat", &["file.txt"])),
                right: Box::new(cmd("grep", &["foo"])),
            }
        );
    }

    #[test]
    fn test_triple_pipe() {
        let expr = parse("cat f | grep x | wc").unwrap();
        assert_eq!(
            expr,
            Expr::Pipe {
                left: Box::new(Expr::Pipe {
                    left: Box::new(cmd("cat", &["f"])),
                    right: Box::new(cmd("grep", &["x"])),
                }),
                right: Box::new(cmd("wc", &[])),
            }
        );
    }

    #[test]
    fn test_redirect_out() {
        let expr = parse("echo hello > out.txt").unwrap();
        assert_eq!(
            expr,
            Expr::Redirect {
                expr: Box::new(cmd("echo", &["hello"])),
                file: Word(vec![WordPart::Literal("out.txt".to_string())]),
                mode: RedirectMode::Overwrite,
            }
        );
    }

    #[test]
    fn test_redirect_append() {
        let expr = parse("echo hi >> out.txt").unwrap();
        assert_eq!(
            expr,
            Expr::Redirect {
                expr: Box::new(cmd("echo", &["hi"])),
                file: Word(vec![WordPart::Literal("out.txt".to_string())]),
                mode: RedirectMode::Append,
            }
        );
    }

    #[test]
    fn test_and_chain() {
        let expr = parse("mkdir test && cd test").unwrap();
        assert_eq!(
            expr,
            Expr::And {
                left: Box::new(cmd("mkdir", &["test"])),
                right: Box::new(cmd("cd", &["test"])),
            }
        );
    }

    #[test]
    fn test_or_chain() {
        let expr = parse("false || echo fallback").unwrap();
        assert_eq!(
            expr,
            Expr::Or {
                left: Box::new(cmd("false", &[])),
                right: Box::new(cmd("echo", &["fallback"])),
            }
        );
    }

    #[test]
    fn test_semicolon_sequence() {
        let expr = parse("echo a ; echo b").unwrap();
        assert_eq!(
            expr,
            Expr::Sequence {
                left: Box::new(cmd("echo", &["a"])),
                right: Box::new(cmd("echo", &["b"])),
            }
        );
    }

    #[test]
    fn test_subshell() {
        let expr = parse("(echo hello)").unwrap();
        assert_eq!(
            expr,
            Expr::Subshell {
                expr: Box::new(cmd("echo", &["hello"])),
            }
        );
    }

    #[test]
    fn test_double_quoted_string() {
        let expr = parse(r#"echo "hello world""#).unwrap();
        assert_eq!(
            expr,
            Expr::Command {
                name: Word(vec![WordPart::Literal("echo".to_string())]),
                args: vec![Word(vec![WordPart::Literal("hello world".to_string())])],
            }
        );
    }

    #[test]
    fn test_single_quoted_string() {
        let expr = parse("echo 'hello world'").unwrap();
        assert_eq!(
            expr,
            Expr::Command {
                name: Word(vec![WordPart::Literal("echo".to_string())]),
                args: vec![Word(vec![WordPart::Literal("hello world".to_string())])],
            }
        );
    }

    #[test]
    fn test_variable_expansion() {
        let expr = parse("echo $HOME").unwrap();
        assert_eq!(
            expr,
            Expr::Command {
                name: Word(vec![WordPart::Literal("echo".to_string())]),
                args: vec![Word(vec![WordPart::Variable("HOME".to_string())])],
            }
        );
    }

    #[test]
    fn test_command_substitution() {
        let expr = parse("echo $(pwd)").unwrap();
        assert_eq!(
            expr,
            Expr::Command {
                name: Word(vec![WordPart::Literal("echo".to_string())]),
                args: vec![Word(vec![WordPart::CommandSubst(Box::new(cmd(
                    "pwd",
                    &[]
                )))])],
            }
        );
    }

    #[test]
    fn test_empty_input_error() {
        assert!(parse("").is_err());
    }

    #[test]
    fn test_complex_pipeline() {
        // cat file.txt | grep foo | sort
        let expr = parse("cat file.txt | grep foo | sort").unwrap();
        assert_eq!(
            expr,
            Expr::Pipe {
                left: Box::new(Expr::Pipe {
                    left: Box::new(cmd("cat", &["file.txt"])),
                    right: Box::new(cmd("grep", &["foo"])),
                }),
                right: Box::new(cmd("sort", &[])),
            }
        );
    }
}
