//! Abstract Syntax Tree types for Nash shell expressions.

/// A single word in the shell, which may contain variable expansions,
/// command substitutions, and literal text.
#[derive(Debug, Clone, PartialEq)]
pub struct Word(pub Vec<WordPart>);

/// One part of a compound word.
#[derive(Debug, Clone, PartialEq)]
pub enum WordPart {
    /// Plain literal text (possibly from a quoted string).
    Literal(String),
    /// `$VARNAME` or `${VARNAME}`
    Variable(String),
    /// `$(...)` — command substitution
    CommandSubst(Box<Expr>),
}

/// How an I/O redirection operates.
#[derive(Debug, Clone, PartialEq)]
pub enum RedirectMode {
    /// `>` — truncate/create and write
    Overwrite,
    /// `>>` — append
    Append,
    /// `<` — read from file
    Input,
}

/// Top-level shell expression node.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A simple command with its arguments.
    Command { name: Word, args: Vec<Word> },

    /// `left | right`
    Pipe { left: Box<Expr>, right: Box<Expr> },

    /// File redirection attached to an expression.
    Redirect {
        expr: Box<Expr>,
        file: Word,
        mode: RedirectMode,
    },

    /// `left ; right` — always run both
    Sequence { left: Box<Expr>, right: Box<Expr> },

    /// `left && right` — run right only if left succeeds
    And { left: Box<Expr>, right: Box<Expr> },

    /// `left || right` — run right only if left fails
    Or { left: Box<Expr>, right: Box<Expr> },

    /// `( expr )` — subshell grouping
    Subshell { expr: Box<Expr> },
}
