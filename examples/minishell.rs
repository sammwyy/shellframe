use shellframe::{Shell, Context, Output};
use indexmap::IndexMap;
use std::io::{self, Write};

fn main() -> anyhow::Result<()> {
    let mut env = IndexMap::new();
    env.insert("USER".to_string(), "example_user".to_string());
    
    let ctx = Context::new("/".to_string(), env, ());
    let mut shell = Shell::new(ctx);

    // Register a builtin
    shell.register_builtin("exit", |_, _, _| {
        std::process::exit(0);
    });

    shell.register_builtin("echo", |args, _, _| {
        Ok(Output::success(format!("{}\n", args.join(" "))))
    });

    shell.register_builtin("cd", |args, ctx, _| {
        let target = args.first().cloned().unwrap_or_else(|| "/".to_string());
        // In this simple example, we don't actually change the host directory,
        // we just update our internal state.
        ctx.set_cwd(target);
        Ok(Output::success("".into()))
    });

    // Register a hook for external commands (real shell behavior)
    shell.set_hook(|name, args, context, _| {
        use std::process::Command;
        let mut cmd = Command::new(name);
        cmd.args(args);
        cmd.current_dir(context.get_cwd());
        cmd.envs(&context.env);

        match cmd.output() {
            Ok(output) => Ok(Output::new(
                output.status.code().unwrap_or(0),
                String::from_utf8_lossy(&output.stdout).to_string(),
                String::from_utf8_lossy(&output.stderr).to_string(),
            )),
            Err(e) => Ok(Output::error(1, "".into(), format!("{}: {}\n", name, e))),
        }
    });

    // Set a custom prompter
    shell.set_prompter(|ctx| {
        format!("\x1b[32m{}\x1b[0m \x1b[34m$\x1b[0m ", ctx.get_cwd())
    });

    println!("Welcome to Minishell! Type 'exit' to quit.");

    loop {
        print!("{}", shell.prompt());
        io::stdout().flush()?;

        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            break;
        }

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        match shell.execute(input) {
            Ok(output) => {
                print!("{}", output.stdout);
                eprint!("{}", output.stderr);
            }
            Err(e) => {
                eprintln!("Error: {}", e);
            }
        }
    }

    Ok(())
}
