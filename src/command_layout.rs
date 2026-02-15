use std::collections::{BTreeSet, HashMap};

use crate::model::{CommandSpec, Plan};

/// Per-package artifact layout requirements derived from captured Cargo units.
///
/// `target_triples` enumerates explicit `--target` triples seen in package commands.
/// `needs_host_artifacts` indicates whether host-layout artifacts (no target triple path)
/// are required for this package, e.g. for proc-macro/custom-build host compilations.
#[derive(Debug, Clone)]
pub struct PackageLayoutRequirements {
    pub target_triples: Vec<String>,
    pub needs_host_artifacts: bool,
}

/// Build host/target layout requirements keyed by `PlanPackage.key`.
///
/// The inference is unit-aware and prioritizes Cargo unit metadata for host-only kinds.
pub fn package_layout_by_key(plan: &Plan) -> HashMap<String, PackageLayoutRequirements> {
    let has_explicit_target_args = plan.units.iter().any(|unit| unit.target_triple.is_some());
    let mut host_flags: HashMap<String, bool> = plan
        .packages
        .iter()
        .map(|package| (package.key.clone(), false))
        .collect();
    let mut target_sets: HashMap<String, BTreeSet<String>> = plan
        .packages
        .iter()
        .map(|package| (package.key.clone(), BTreeSet::new()))
        .collect();

    for unit in &plan.units {
        let is_host_forced =
            unit.target_kind == "custom-build"
                || unit.target_kind == "proc-macro"
                || unit.compile_mode.contains("build-script");

        if is_host_forced {
            host_flags.insert(unit.package_key.clone(), true);
            continue;
        }

        let inferred_target = unit.target_triple.clone().or_else(|| {
            if has_explicit_target_args {
                None
            } else {
                plan.target_triple.clone()
            }
        });

        if let Some(triple) = inferred_target {
            target_sets
                .entry(unit.package_key.clone())
                .or_default()
                .insert(triple);
        } else {
            host_flags.insert(unit.package_key.clone(), true);
        }
    }

    plan.packages
        .iter()
        .map(|package| {
            let target_triples = target_sets
                .remove(&package.key)
                .unwrap_or_default()
                .into_iter()
                .collect::<Vec<_>>();
            let needs_host_artifacts = host_flags.remove(&package.key).unwrap_or(false);
            (
                package.key.clone(),
                PackageLayoutRequirements {
                    target_triples,
                    needs_host_artifacts,
                },
            )
        })
        .collect()
}

/// Extract `--target` triple from a captured rustc command if present.
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
    use crate::model::{CommandEnv, CommandSpec, Plan, PlanPackage, Unit};

    use super::{command_target_triple, package_layout_by_key};

    fn command(args: &[&str]) -> CommandSpec {
        CommandSpec {
            cwd: None,
            env: Vec::<CommandEnv>::new(),
            program: "rustc".to_string(),
            args: args.iter().map(|value| (*value).to_string()).collect(),
        }
    }

    fn unit_for(
        package_key: &str,
        target_kind: &str,
        compile_mode: &str,
        args: &[&str],
    ) -> Unit {
        let command = command(args);
        Unit {
            unit_id: "unit".to_string(),
            package_key: package_key.to_string(),
            package_name: "pkg".to_string(),
            package_version: "0.1.0".to_string(),
            target_name: "pkg".to_string(),
            target_kind: target_kind.to_string(),
            compile_mode: compile_mode.to_string(),
            target_triple: command_target_triple(&command),
            build_script_binary: None,
            package_dependencies: Vec::new(),
            command,
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
    fn unit_layout_marks_proc_macro_as_host() {
        let package_key = "pkg v0.1.0 (/tmp/pkg)";
        let plan = Plan {
            workspace_root: "/tmp".to_string(),
            manifest_path: "/tmp/Cargo.toml".to_string(),
            cargo_home: "/tmp/ch".to_string(),
            target_dir: "/tmp/target".to_string(),
            target_triple: None,
            packages: vec![PlanPackage {
                key: package_key.to_string(),
                name: "pkg".to_string(),
                version: "0.1.0".to_string(),
                source: "/tmp/pkg".to_string(),
                manifest_path: "/tmp/pkg/Cargo.toml".to_string(),
                cargo_home_rel_manifest_path: None,
                lock_checksum: None,
                workspace_member: true,
                dependencies: Vec::new(),
            }],
            units: vec![
                unit_for(
                    package_key,
                    "proc-macro",
                    "build",
                    &["--target", "aarch64-unknown-linux-gnu"],
                ),
                unit_for(
                    package_key,
                    "lib",
                    "build",
                    &["--target", "aarch64-unknown-linux-gnu"],
                ),
            ],
        };

        let layout = package_layout_by_key(&plan);
        let package = layout.get(package_key).expect("layout exists");
        assert!(package.needs_host_artifacts);
        assert_eq!(
            package.target_triples,
            vec!["aarch64-unknown-linux-gnu".to_string()]
        );
    }

    #[test]
    fn unit_layout_keeps_pure_target_builds_off_host_layout() {
        let package_key = "pkg v0.1.0 (/tmp/pkg)";
        let plan = Plan {
            workspace_root: "/tmp".to_string(),
            manifest_path: "/tmp/Cargo.toml".to_string(),
            cargo_home: "/tmp/ch".to_string(),
            target_dir: "/tmp/target".to_string(),
            target_triple: None,
            packages: vec![PlanPackage {
                key: package_key.to_string(),
                name: "pkg".to_string(),
                version: "0.1.0".to_string(),
                source: "/tmp/pkg".to_string(),
                manifest_path: "/tmp/pkg/Cargo.toml".to_string(),
                cargo_home_rel_manifest_path: None,
                lock_checksum: None,
                workspace_member: true,
                dependencies: Vec::new(),
            }],
            units: vec![unit_for(
                package_key,
                "bin",
                "build",
                &["--target", "x86_64-unknown-linux-gnu"],
            )],
        };

        let layout = package_layout_by_key(&plan);
        let package = layout.get(package_key).expect("layout exists");
        assert!(!package.needs_host_artifacts);
        assert_eq!(
            package.target_triples,
            vec!["x86_64-unknown-linux-gnu".to_string()]
        );
    }
}
