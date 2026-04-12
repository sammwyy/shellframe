//! Lexer — converts a raw shell string into a flat token stream.
//!
//! The lexer handles quoting, variable expansion, and command substitution
//! so the parser only needs to deal with structured tokens.

use super::ast::{Word, WordPart};
use super::ParseError;

/// Tokens emitted by the lexer.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// A (possibly compound) word.
    Word(Word),
    /// `|`
    Pipe,
    /// `&&`
    And,
    /// `||`
    Or,
    /// `;`
    Semi,
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `>`
    RedirectOut,
    /// `>>`
    RedirectAppend,
    /// `<`
    RedirectIn,
    /// End of input.
    Eof,
}

pub struct Lexer<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    /// Tokenize the full input.
    pub fn tokenize(mut self) -> Result<Vec<Token>, ParseError> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace();
            if self.pos >= self.src.len() {
                tokens.push(Token::Eof);
                break;
            }

            let ch = self.current_char();

            // comments — skip rest of line but still emit Eof
            if ch == '#' {
                tokens.push(Token::Eof);
                break;
            }

            let tok = match ch {
                '\n' => {
                    self.advance();
                    Token::Semi
                }
                '|' => {
                    self.advance();
                    if self.current_char() == '|' {
                        self.advance();
                        Token::Or
                    } else {
                        Token::Pipe
                    }
                }
                '&' => {
                    self.advance();
                    if self.current_char() == '&' {
                        self.advance();
                        Token::And
                    } else {
                        // bare `&` — treat as `;` for simplicity (background not supported)
                        Token::Semi
                    }
                }
                ';' => {
                    self.advance();
                    Token::Semi
                }
                '(' => {
                    self.advance();
                    Token::LParen
                }
                ')' => {
                    self.advance();
                    Token::RParen
                }
                '>' => {
                    self.advance();
                    if self.current_char() == '>' {
                        self.advance();
                        Token::RedirectAppend
                    } else {
                        Token::RedirectOut
                    }
                }
                '<' => {
                    self.advance();
                    Token::RedirectIn
                }
                _ => {
                    // word
                    let word = self.read_word()?;
                    Token::Word(word)
                }
            };

            tokens.push(tok);
        }
        Ok(tokens)
    }

    // ─── helpers ─────────────────────────────────────────────────────────────

    fn current_char(&self) -> char {
        self.src[self.pos..].chars().next().unwrap_or('\0')
    }

    fn advance(&mut self) {
        if self.pos < self.src.len() {
            let ch = self.current_char();
            self.pos += ch.len_utf8();
        }
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.src.len() {
            let ch = self.current_char();
            if ch == ' ' || ch == '\t' || ch == '\r' {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn at_word_boundary(&self) -> bool {
        if self.pos >= self.src.len() {
            return true;
        }
        matches!(
            self.current_char(),
            ' ' | '\t' | '\r' | '\n' | '|' | '&' | ';' | '(' | ')' | '>' | '<'
        )
    }

    /// Read a (potentially compound) word until a word boundary.
    fn read_word(&mut self) -> Result<Word, ParseError> {
        let mut parts: Vec<WordPart> = Vec::new();
        let mut literal_buf = String::new();

        while !self.at_word_boundary() {
            let ch = self.current_char();
            match ch {
                '\'' => {
                    // single-quoted: no expansions
                    self.advance(); // opening '
                    while self.pos < self.src.len() && self.current_char() != '\'' {
                        literal_buf.push(self.current_char());
                        self.advance();
                    }
                    if self.pos >= self.src.len() {
                        return Err(ParseError::Unmatched('\''));
                    }
                    self.advance(); // closing '
                }
                '"' => {
                    // double-quoted: variable/command-subst expansions are active
                    self.advance(); // opening "
                    while self.pos < self.src.len() && self.current_char() != '"' {
                        if self.current_char() == '$' {
                            // flush literal so far
                            if !literal_buf.is_empty() {
                                parts.push(WordPart::Literal(std::mem::take(&mut literal_buf)));
                            }
                            let part = self.read_dollar()?;
                            parts.push(part);
                        } else if self.current_char() == '\\' {
                            self.advance();
                            let escaped = self.current_char();
                            self.advance();
                            literal_buf.push(escaped);
                        } else {
                            literal_buf.push(self.current_char());
                            self.advance();
                        }
                    }
                    if self.pos >= self.src.len() {
                        return Err(ParseError::Unmatched('"'));
                    }
                    self.advance(); // closing "
                }
                '$' => {
                    if !literal_buf.is_empty() {
                        parts.push(WordPart::Literal(std::mem::take(&mut literal_buf)));
                    }
                    let part = self.read_dollar()?;
                    parts.push(part);
                }
                '\\' => {
                    self.advance();
                    if self.pos < self.src.len() {
                        let next = self.current_char();
                        if next == '\n' {
                            self.advance();
                        } else {
                            literal_buf.push(next);
                            self.advance();
                        }
                    }
                }
                _ => {
                    literal_buf.push(ch);
                    self.advance();
                }
            }
        }

        if !literal_buf.is_empty() {
            parts.push(WordPart::Literal(literal_buf));
        }

        Ok(Word(parts))
    }

    /// Called after seeing `$` — reads a variable or `$(...)`.
    fn read_dollar(&mut self) -> Result<WordPart, ParseError> {
        self.advance(); // consume '$'

        if self.pos >= self.src.len() {
            return Ok(WordPart::Literal("$".to_string()));
        }

        match self.current_char() {
            '(' => {
                // command substitution: $(...)
                self.advance(); // consume '('
                let sub_src = self.read_until_close_paren()?;
                // recursively parse the substituted command
                let expr = super::parse(&sub_src).map_err(|_| ParseError::Unmatched('('))?;
                Ok(WordPart::CommandSubst(Box::new(expr)))
            }
            '{' => {
                // ${VARNAME}
                self.advance(); // consume '{'
                let name = self.read_identifier();
                if self.pos < self.src.len() && self.current_char() == '}' {
                    self.advance();
                }
                Ok(WordPart::Variable(name))
            }
            c if c.is_alphabetic() || c == '_' => {
                let name = self.read_identifier();
                Ok(WordPart::Variable(name))
            }
            _ => Ok(WordPart::Literal("$".to_string())),
        }
    }

    fn read_identifier(&mut self) -> String {
        let mut name = String::new();
        while self.pos < self.src.len() {
            let c = self.current_char();
            if c.is_alphanumeric() || c == '_' {
                name.push(c);
                self.advance();
            } else {
                break;
            }
        }
        name
    }

    /// Read characters until matching `)`, handling nesting.
    fn read_until_close_paren(&mut self) -> Result<String, ParseError> {
        let mut depth = 1usize;
        let mut buf = String::new();
        while self.pos < self.src.len() {
            let c = self.current_char();
            self.advance();
            match c {
                '(' => {
                    depth += 1;
                    buf.push(c);
                }
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(buf);
                    }
                    buf.push(c);
                }
                _ => buf.push(c),
            }
        }
        Err(ParseError::Unmatched('('))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ast::WordPart;

    fn lit(s: &str) -> Token {
        Token::Word(Word(vec![WordPart::Literal(s.to_string())]))
    }

    fn tokenize(s: &str) -> Vec<Token> {
        Lexer::new(s).tokenize().unwrap()
    }

    #[test]
    fn test_simple_words() {
        let toks = tokenize("echo hello world");
        assert_eq!(
            toks,
            vec![lit("echo"), lit("hello"), lit("world"), Token::Eof]
        );
    }

    #[test]
    fn test_pipe_token() {
        let toks = tokenize("a | b");
        assert_eq!(toks, vec![lit("a"), Token::Pipe, lit("b"), Token::Eof]);
    }

    #[test]
    fn test_and_token() {
        let toks = tokenize("a && b");
        assert_eq!(toks, vec![lit("a"), Token::And, lit("b"), Token::Eof]);
    }

    #[test]
    fn test_or_token() {
        let toks = tokenize("a || b");
        assert_eq!(toks, vec![lit("a"), Token::Or, lit("b"), Token::Eof]);
    }

    #[test]
    fn test_redirect_out() {
        let toks = tokenize("echo > file");
        assert_eq!(
            toks,
            vec![lit("echo"), Token::RedirectOut, lit("file"), Token::Eof]
        );
    }

    #[test]
    fn test_redirect_append() {
        let toks = tokenize("echo >> file");
        assert_eq!(
            toks,
            vec![lit("echo"), Token::RedirectAppend, lit("file"), Token::Eof]
        );
    }

    #[test]
    fn test_single_quoted() {
        let toks = tokenize("echo 'hello world'");
        assert_eq!(toks, vec![lit("echo"), lit("hello world"), Token::Eof]);
    }

    #[test]
    fn test_double_quoted() {
        let toks = tokenize(r#"echo "hello world""#);
        assert_eq!(toks, vec![lit("echo"), lit("hello world"), Token::Eof]);
    }

    #[test]
    fn test_variable() {
        let toks = tokenize("echo $HOME");
        assert_eq!(
            toks,
            vec![
                lit("echo"),
                Token::Word(Word(vec![WordPart::Variable("HOME".to_string())])),
                Token::Eof
            ]
        );
    }

    #[test]
    fn test_parens() {
        let toks = tokenize("(echo)");
        assert_eq!(
            toks,
            vec![Token::LParen, lit("echo"), Token::RParen, Token::Eof]
        );
    }

    #[test]
    fn test_comment() {
        let toks = tokenize("echo hello # this is a comment");
        assert_eq!(toks, vec![lit("echo"), lit("hello"), Token::Eof]);
    }
}
