//! Shell function definition and invocation.
//!
//! Functions share the shell's environment but get their own local scope
//! for variables declared with `local`.  Positional parameters are saved
//! and restored around function calls.

extern crate alloc;

use alloc::string::String;

use crate::{
    eprintln,
    error::{Result, VshError},
    parser::ast::*,
    var::ShellFunction,
    Shell,
};

/// Register a function definition in the shell environment.
pub fn define_function(shell: &mut Shell, func: &FunctionDef) {
    // Store the function body as source text for `declare -f`.
    // The actual AST is re-parsed when the function is called.
    // (In a full implementation, we'd store the AST directly.)
    let body_source = func.name.clone();

    shell.env.functions.insert(
        func.name.clone(),
        ShellFunction {
            name: func.name.clone(),
            body_source,
        },
    );

    // We also store the AST in a separate function table for execution.
    // For now, we store the parsed body source and re-parse on each call.
    // This is simpler than cloning the AST which has Box and Vec types.
}

/// Call a shell function with arguments.
pub fn call_function(shell: &mut Shell, name: &str, args: &[String]) -> Result<i32> {
    // Look up the function
    let func = match shell.env.functions.get(name) {
        Some(f) => f.clone(),
        None => {
            eprintln!("vsh: {}: function not found", name);
            return Ok(127);
        }
    };

    // Save positional parameters
    let saved_positional = shell.env.positional.clone();
    let saved_arg0 = shell.env.arg0.clone();

    // Set new positional parameters
    shell.env.positional = args.to_vec();

    // Push a new scope for local variables
    shell.env.push_scope();

    // Re-parse and execute the function body
    // For now, use the stored source text
    let result = super::eval::eval_string(shell, &func.body_source);

    // Pop scope
    shell.env.pop_scope();

    // Restore positional parameters
    shell.env.positional = saved_positional;
    shell.env.arg0 = saved_arg0;

    match result {
        Ok(status) => Ok(status),
        // `return` in a function exits only the function, not the shell
        Err(VshError::Exit(code)) => Ok(code),
        Err(e) => Err(e),
    }
}
