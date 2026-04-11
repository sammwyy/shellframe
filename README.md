# Shellframe

**Shellframe** is a flexible and extensible framework for building shell environments in Rust. It abstracts the heavy lifting of parsing (with a built-in bash-like AST parser) and execution flow, allowing you to easily build sandboxed environments, custom CLI tools, or even fully-fledged system shells.

## Features

- **No External Parser Dependencies:** Includes a hand-written recursive-descent parser that supports a subset of bash syntax (pipelines, redirections, sequences, subshells, boolean AND/OR, and variable expansions).
- **Flexible Execution Context:** Bring your own state (`T`) into the execution context, allowing per-session variables, virtual file systems, or custom data structures.
- **Pluggable Architecture:**
  - Build and register your own built-in commands natively in Rust.
  - Set a `hook` to catch unknown commands (ideal for delegating to system binaries or virtual executables).
  - Customize redirection handlers (e.g., to read/write from a host file system or a virtual one).
  - Customize the shell prompt with a simple callback.

## Getting Started

Add `shellframe` to your project and start building your custom shell.

### Minimal Example

Here is a minimal in-memory example to demonstrate the framework's simplicity.

```rust
use shellframe::{Shell, Context, Output};
use indexmap::IndexMap;
use std::io::{self, Write};

fn main() -> anyhow::Result<()> {
    // 1. Create your environment and Context
    let mut env = IndexMap::new();
    env.insert("USER".to_string(), "example_user".to_string());
    
    // We pass `()` as our generic state type since we don't need custom state here.
    let ctx = Context::new("/".to_string(), env, ());
    
    // 2. Instantiate the Shell
    let mut shell = Shell::new(ctx);

    // 3. Register native builtins
    shell.register_builtin("echo", |args, _ctx, _stdin| {
        Ok(Output::success(format!("{}\n", args.join(" "))))
    });

    shell.register_builtin("exit", |_args, _ctx, _stdin| {
        std::process::exit(0);
    });

    // 4. (Optional) Set a fallback hook for unknown commands
    shell.set_hook(|name, _args, _context, _stdin| {
        Ok(Output::error(127, "".into(), format!("{}: command not found\n", name)))
    });

    // 5. Read, Evaluate, Print Loop (REPL)
    loop {
        print!("{} $ ", shell.context.get_cwd());
        io::stdout().flush()?;

        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            break;
        }

        let input = input.trim();
        if input.is_empty() { continue; }

        match shell.execute(input) {
            Ok(output) => {
                print!("{}", output.stdout);
                eprint!("{}", output.stderr);
            }
            Err(e) => eprintln!("Parse error: {}", e),
        }
    }

    Ok(())
}
```

## Advanced Usage

### Custom State
To attach custom state (like a virtual file system or database connection), define your struct and pass it when creating the context:

```rust
struct MyState {
    pub vfs: VirtualFileSystem,
}

let ctx = Context::new("/home".into(), env_map, MyState::new());
let mut shell = Shell::new(ctx);

// Builtins can access the state mutably:
shell.register_builtin("ls", |_args, ctx, _stdin| {
    // ctx.state is of type `MyState`
    let files = ctx.state.vfs.list(ctx.get_cwd());
    // ...
});
```

### Inheriting Host Environment
If you are building a host system shell, you can use the helper method `inherit_system_env` to automatically populate the shell's environment map and `PWD` from the host operating system:

```rust
let mut ctx = Context::new(cwd, indexmap::IndexMap::new(), ());
ctx.inherit_system_env();
```

### Redirection Handling
You can control what happens when a user types `echo hello > file.txt` by providing a redirection handler:

```rust
use shellframe::RedirectMode;

shell.set_redirect_handler(|sh, expr, file, mode, stdin| {
    // Evaluate the left-side expression first
    let output = sh.eval(expr, stdin)?;
    
    // Handle the output based on Virtual or Host file systems
    match mode {
        RedirectMode::Overwrite => { /* Write output.stdout to `file` */ },
        RedirectMode::Append => { /* Append output.stdout to `file` */ },
        RedirectMode::Input => { /* Read `file` and pass as stdin to `expr` */ }
    }
    
    Ok(Output::success("".into()))
});
```

## Contributing
Contributions are welcome! Feel free to open issues or submit pull requests with bug fixes or new features.

## License
MIT
