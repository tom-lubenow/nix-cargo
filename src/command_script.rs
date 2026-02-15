use std::fmt::Write;

use crate::model::Unit;
use crate::nix_string::{shell_array_literal, shell_single_quote};

pub(crate) fn render_command_script(units: &[Unit]) -> String {
    let mut script = String::new();

    for (index, unit) in units.iter().enumerate() {
        let command = &unit.command;
        let is_build_script_compile = if is_build_script_compile(unit) { 1 } else { 0 };
        let build_script_binary = unit.build_script_binary.clone().unwrap_or_default();
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
            "  run_cargo_cmd {is_build_script_compile} {} {} {} {args_var} {env_var}",
            shell_single_quote(&build_script_binary),
            shell_single_quote(&cwd),
            shell_single_quote(&command.program),
        );
        let _ = writeln!(script, "  unset {args_var} {env_var}");
        script.push_str("}\n");
    }

    script
}

fn is_build_script_compile(unit: &Unit) -> bool {
    unit.target_kind == "custom-build" && unit.compile_mode == "Build"
}
