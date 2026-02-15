use std::fmt::Write;

use crate::model::CommandSpec;
use crate::nix_string::{shell_array_literal, shell_single_quote};

pub(crate) fn render_command_script(commands: &[CommandSpec]) -> String {
    let mut script = String::new();

    for (index, command) in commands.iter().enumerate() {
        let args_var = format!("cmd_args_{index}");
        let env_var = format!("cmd_env_{index}");
        let cwd = command.cwd.clone().unwrap_or_default();
        let env_pairs = command
            .env
            .iter()
            .map(|entry| format!("{}={}", entry.key, entry.value))
            .collect::<Vec<_>>();

        script.push_str("{\n");
        let _ = writeln!(
            script,
            "  declare -a {args_var}=({})",
            shell_array_literal(&command.args)
        );
        let _ = writeln!(
            script,
            "  declare -a {env_var}=({})",
            shell_array_literal(&env_pairs)
        );
        let _ = writeln!(
            script,
            "  run_cargo_cmd {} {} {args_var} {env_var}",
            shell_single_quote(&cwd),
            shell_single_quote(&command.program),
        );
        let _ = writeln!(script, "  unset {args_var} {env_var}");
        script.push_str("}\n");
    }

    script
}

