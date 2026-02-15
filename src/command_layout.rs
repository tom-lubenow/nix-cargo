use std::collections::BTreeSet;

use crate::model::CommandSpec;

pub fn package_target_triples(commands: &[CommandSpec]) -> Vec<String> {
    commands
        .iter()
        .filter_map(command_target_triple)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub fn package_needs_host_artifacts(commands: &[CommandSpec]) -> bool {
    commands
        .iter()
        .any(|command| command_target_triple(command).is_none())
}

pub fn command_target_triple(command: &CommandSpec) -> Option<String> {
    let mut args = command.args.iter();
    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--target=") {
            if !value.is_empty() {
                return Some(value.to_string());
            }
            continue;
        }

        if arg == "--target" {
            let next = args.next()?;
            if !next.is_empty() {
                return Some(next.to_string());
            }
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::model::{CommandEnv, CommandSpec};

    use super::{command_target_triple, package_needs_host_artifacts, package_target_triples};

    fn command(args: &[&str]) -> CommandSpec {
        CommandSpec {
            cwd: None,
            env: Vec::<CommandEnv>::new(),
            program: "rustc".to_string(),
            args: args.iter().map(|value| (*value).to_string()).collect(),
        }
    }

    #[test]
    fn parses_inline_target_arg() {
        let triple = command_target_triple(&command(&["--crate-name", "x", "--target=aarch64-unknown-linux-gnu"]));
        assert_eq!(triple.as_deref(), Some("aarch64-unknown-linux-gnu"));
    }

    #[test]
    fn parses_split_target_arg() {
        let triple = command_target_triple(&command(&["--target", "x86_64-unknown-linux-gnu"]));
        assert_eq!(triple.as_deref(), Some("x86_64-unknown-linux-gnu"));
    }

    #[test]
    fn computes_target_set_and_host_requirement() {
        let commands = vec![
            command(&["--crate-name", "host-tool"]),
            command(&["--target", "x86_64-unknown-linux-gnu"]),
            command(&["--target=aarch64-unknown-linux-gnu"]),
        ];
        assert_eq!(
            package_target_triples(&commands),
            vec![
                "aarch64-unknown-linux-gnu".to_string(),
                "x86_64-unknown-linux-gnu".to_string()
            ]
        );
        assert!(package_needs_host_artifacts(&commands));
    }
}

