pub mod parser;

use anyhow::Result;
use indexmap::IndexMap;
pub use parser::ast::{Expr, RedirectMode, Word, WordPart};

#[derive(Debug, Clone)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl Output {
    pub fn new(exit_code: i32, stdout: String, stderr: String) -> Self {
        Self {
            stdout,
            stderr,
            exit_code,
        }
    }

    pub fn success(stdout: String) -> Self {
        Self::new(0, stdout, String::new())
    }

    pub fn error(exit_code: i32, stdout: String, stderr: String) -> Self {
        Self::new(exit_code, stdout, stderr)
    }

    pub fn is_success(&self) -> bool {
        self.exit_code == 0
    }
}

pub type BuiltinFn<T> =
    std::sync::Arc<dyn Fn(&[String], &mut Context<T>, &str) -> Result<Output> + Send + Sync>;
pub type HookFn<T> =
    std::sync::Arc<dyn Fn(&str, &[String], &mut Context<T>, &str) -> Result<Output> + Send + Sync>;
pub type PromptFn<T> = std::sync::Arc<dyn Fn(&Context<T>) -> String + Send + Sync>;
// Redirection hook receives the evaluated (expanded) filename
pub type RedirectFn<T> = std::sync::Arc<
    dyn Fn(&mut Shell<T>, &Expr, &str, &RedirectMode, &str) -> Result<Output> + Send + Sync,
>;

pub struct Context<T = ()> {
    cwd: String,
    pub env: IndexMap<String, String>,
    pub last_exit_code: i32,
    pub state: T,
}

impl<T> Context<T> {
    pub fn new(cwd: String, mut env: IndexMap<String, String>, state: T) -> Self {
        if !env.contains_key("PWD") {
            env.insert("PWD".into(), cwd.clone());
        }
        Self {
            cwd,
            env,
            last_exit_code: 0,
            state,
        }
    }

    /// Inherit all environment variables from the host system.
    pub fn inherit_system_env(&mut self) {
        for (k, v) in std::env::vars() {
            self.env.insert(k, v);
        }
        if let Some(pwd) = self.env.get("PWD") {
            self.cwd = pwd.clone();
        }
    }

    pub fn get_cwd(&self) -> &str {
        &self.cwd
    }

    pub fn set_cwd(&mut self, new_cwd: String) {
        self.cwd = new_cwd.clone();
        self.env.insert("PWD".into(), new_cwd);
    }
}

pub struct Shell<T = ()> {
    pub context: Context<T>,
    builtins: IndexMap<String, BuiltinFn<T>>,
    hook: Option<HookFn<T>>,
    prompter: Option<PromptFn<T>>,
    redirect_handler: Option<RedirectFn<T>>,
}

impl<T> Shell<T> {
    pub fn new(context: Context<T>) -> Self {
        Self {
            context,
            builtins: IndexMap::new(),
            hook: None,
            prompter: None,
            redirect_handler: None,
        }
    }

    pub fn register_builtin<F>(&mut self, name: &str, f: F)
    where
        F: Fn(&[String], &mut Context<T>, &str) -> Result<Output> + Send + Sync + 'static,
    {
        self.builtins
            .insert(name.to_string(), std::sync::Arc::new(f));
    }

    pub fn set_hook<F>(&mut self, hook: F)
    where
        F: Fn(&str, &[String], &mut Context<T>, &str) -> Result<Output> + Send + Sync + 'static,
    {
        self.hook = Some(std::sync::Arc::new(hook));
    }

    pub fn set_prompter<F>(&mut self, prompter: F)
    where
        F: Fn(&Context<T>) -> String + Send + Sync + 'static,
    {
        self.prompter = Some(std::sync::Arc::new(prompter));
    }

    pub fn set_redirect_handler<F>(&mut self, handler: F)
    where
        F: Fn(&mut Shell<T>, &Expr, &str, &RedirectMode, &str) -> Result<Output>
            + Send
            + Sync
            + 'static,
    {
        self.redirect_handler = Some(std::sync::Arc::new(handler));
    }

    pub fn prompt(&self) -> String {
        if let Some(p) = self.prompter.clone() {
            p(&self.context)
        } else {
            format!("{}> ", self.context.get_cwd())
        }
    }

    pub fn execute(&mut self, input: &str) -> Result<Output> {
        let expr = parser::parse(input)?;
        self.eval(&expr, "")
    }

    pub fn eval(&mut self, expr: &Expr, stdin: &str) -> Result<Output> {
        match expr {
            Expr::Command { name, args } => self.eval_command(name, args, stdin),

            Expr::Pipe { left, right } => {
                let left_out = self.eval(left, stdin)?;
                self.eval(right, &left_out.stdout)
            }

            Expr::Redirect { expr, file, mode } => {
                let file_str = self.expand_word(file)?;
                if let Some(handler) = self.redirect_handler.clone() {
                    handler(self, expr, &file_str, mode, stdin)
                } else {
                    Ok(Output::error(
                        1,
                        "".into(),
                        "Redirection not supported by this shell\n".into(),
                    ))
                }
            }

            Expr::Sequence { left, right } => {
                let _ = self.eval(left, stdin)?;
                self.eval(right, stdin)
            }

            Expr::And { left, right } => {
                let left_out = self.eval(left, stdin)?;
                if left_out.is_success() {
                    self.eval(right, stdin)
                } else {
                    Ok(left_out)
                }
            }

            Expr::Or { left, right } => {
                let left_out = self.eval(left, stdin)?;
                if !left_out.is_success() {
                    self.eval(right, stdin)
                } else {
                    Ok(left_out)
                }
            }

            Expr::Subshell { expr } => {
                let saved_cwd = self.context.get_cwd().to_string();
                let saved_env = self.context.env.clone();
                let result = self.eval(expr, stdin);
                self.context.set_cwd(saved_cwd);
                self.context.env = saved_env;
                result
            }
        }
    }

    fn eval_command(&mut self, name: &Word, args: &[Word], stdin: &str) -> Result<Output> {
        let name_str = self.expand_word(name)?;
        let mut arg_strs = Vec::new();
        for arg in args {
            arg_strs.push(self.expand_word(arg)?);
        }

        if let Some(builtin) = self.builtins.get(&name_str).cloned() {
            let out = builtin(&arg_strs, &mut self.context, stdin)?;
            self.context.last_exit_code = out.exit_code;
            Ok(out)
        } else if let Some(hook) = self.hook.clone() {
            let out = hook(&name_str, &arg_strs, &mut self.context, stdin)?;
            self.context.last_exit_code = out.exit_code;
            Ok(out)
        } else {
            Ok(Output::error(
                127,
                "".into(),
                format!("command not found: {}\n", name_str),
            ))
        }
    }

    pub fn expand_word(&mut self, word: &Word) -> Result<String> {
        let mut result = String::new();
        for part in &word.0 {
            match part {
                WordPart::Literal(s) => result.push_str(s),
                WordPart::Variable(name) => {
                    let val = self.context.env.get(name).cloned().unwrap_or_default();
                    result.push_str(&val);
                }
                WordPart::CommandSubst(expr) => {
                    let output = self.eval(expr, "")?;
                    result.push_str(output.stdout.trim_end_matches('\n'));
                }
            }
        }
        Ok(result)
    }
}
