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

pub type BuiltinFn<T> = fn(args: &[String], context: &mut Context<T>, stdin: &str) -> Result<Output>;
pub type HookFn<T> = fn(name: &str, args: &[String], context: &mut Context<T>, stdin: &str) -> Result<Output>;
pub type PromptFn<T> = fn(context: &Context<T>) -> String;
// Redirection hook receives the evaluated (expanded) filename
pub type RedirectFn<T> = fn(shell: &mut Shell<T>, expr: &Expr, file: &str, mode: &RedirectMode, stdin: &str) -> Result<Output>;

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

    pub fn register_builtin(&mut self, name: &str, f: BuiltinFn<T>) {
        self.builtins.insert(name.to_string(), f);
    }

    pub fn set_hook(&mut self, hook: HookFn<T>) {
        self.hook = Some(hook);
    }

    pub fn set_prompter(&mut self, prompter: PromptFn<T>) {
        self.prompter = Some(prompter);
    }

    pub fn set_redirect_handler(&mut self, handler: RedirectFn<T>) {
        self.redirect_handler = Some(handler);
    }

    pub fn prompt(&self) -> String {
        if let Some(p) = self.prompter {
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
                if let Some(handler) = self.redirect_handler {
                    handler(self, expr, &file_str, mode, stdin)
                } else {
                    Ok(Output::error(1, "".into(), "Redirection not supported by this shell\n".into()))
                }
            }

            Expr::Sequence { left, right } => {
                let _left_out = self.eval(left, stdin)?;
                // For a framework, we just return the final output but many shells would print intermediate output here
                // We'll let the user handle printing if they want, but maybe we should provide a way to 'collect' output?
                // For now, let's just return the last one as per previous logic.
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

        if let Some(builtin) = self.builtins.get(&name_str) {
            let out = builtin(&arg_strs, &mut self.context, stdin)?;
            self.context.last_exit_code = out.exit_code;
            Ok(out)
        } else if let Some(hook) = self.hook {
            let out = hook(&name_str, &arg_strs, &mut self.context, stdin)?;
            self.context.last_exit_code = out.exit_code;
            Ok(out)
        } else {
            Ok(Output::error(127, "".into(), format!("command not found: {}\n", name_str)))
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
