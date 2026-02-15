use std::collections::HashMap;

use crate::command_layout::PackageLayoutRequirements;
use crate::command_script::render_command_script;
use crate::model::{PlanPackage, Unit};

#[derive(Debug, Clone)]
pub(crate) struct RenderedPackagePlan {
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) source: String,
    pub(crate) lock_checksum: Option<String>,
    pub(crate) cargo_home_rel_manifest_path: Option<String>,
    pub(crate) workspace_member: bool,
    pub(crate) dependencies: Vec<String>,
    pub(crate) workspace_source_prefixes: Vec<String>,
    pub(crate) target_triples: Vec<String>,
    pub(crate) needs_host_artifacts: bool,
    pub(crate) command_script: String,
}

pub(crate) fn build_rendered_package_plans(
    ordered_packages: &[PlanPackage],
    units_by_package: &HashMap<String, Vec<Unit>>,
    package_layout: &HashMap<String, PackageLayoutRequirements>,
    source_prefixes_by_package: &HashMap<String, Vec<String>>,
) -> Vec<RenderedPackagePlan> {
    ordered_packages
        .iter()
        .map(|package| {
            let package_units = units_by_package
                .get(package.key.as_str())
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let layout = package_layout.get(package.key.as_str());
            let target_triples = layout
                .map(|layout| layout.target_triples.clone())
                .unwrap_or_default();
            let needs_host_artifacts = layout
                .map(|layout| layout.needs_host_artifacts)
                .unwrap_or(false);
            let workspace_source_prefixes = source_prefixes_by_package
                .get(package.key.as_str())
                .cloned()
                .unwrap_or_default();

            RenderedPackagePlan {
                key: package.key.clone(),
                name: package.name.clone(),
                version: package.version.clone(),
                source: package.source.clone(),
                lock_checksum: package.lock_checksum.clone(),
                cargo_home_rel_manifest_path: package.cargo_home_rel_manifest_path.clone(),
                workspace_member: package.workspace_member,
                dependencies: package.dependencies.clone(),
                workspace_source_prefixes,
                target_triples,
                needs_host_artifacts,
                command_script: render_command_script(package_units),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::command_layout::PackageLayoutRequirements;
    use crate::model::{CommandEnv, CommandSpec, PlanPackage, Unit};

    use super::build_rendered_package_plans;

    fn package(key: &str, member: bool) -> PlanPackage {
        PlanPackage {
            key: key.to_string(),
            name: "pkg".to_string(),
            version: "0.1.0".to_string(),
            source: "/tmp/pkg".to_string(),
            manifest_path: "/tmp/pkg/Cargo.toml".to_string(),
            cargo_home_rel_manifest_path: None,
            lock_checksum: None,
            workspace_member: member,
            dependencies: vec!["dep v0.1.0 (/tmp/dep)".to_string()],
        }
    }

    fn unit_for_package(package_key: &str) -> Unit {
        Unit {
            unit_id: "u1".to_string(),
            package_key: package_key.to_string(),
            package_name: "pkg".to_string(),
            package_version: "0.1.0".to_string(),
            target_name: "pkg".to_string(),
            target_kind: "lib".to_string(),
            compile_mode: "Build".to_string(),
            target_triple: None,
            build_script_binary: None,
            package_dependencies: Vec::new(),
            command: CommandSpec {
                cwd: Some("/tmp/pkg".to_string()),
                env: vec![CommandEnv {
                    key: "RUSTC".to_string(),
                    value: "/nix/store/rustc/bin/rustc".to_string(),
                }],
                program: "rustc".to_string(),
                args: vec!["--crate-name".to_string(), "pkg".to_string()],
            },
        }
    }

    #[test]
    fn renders_defaults_without_optional_maps() {
        let package = package("pkg v0.1.0 (/tmp/pkg)", true);
        let rendered = build_rendered_package_plans(
            std::slice::from_ref(&package),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(rendered.len(), 1);
        assert_eq!(rendered[0].key, package.key);
        assert!(rendered[0].target_triples.is_empty());
        assert!(!rendered[0].needs_host_artifacts);
        assert!(rendered[0].workspace_source_prefixes.is_empty());
        assert!(rendered[0].command_script.is_empty());
    }

    #[test]
    fn carries_layout_and_script_data() {
        let package_key = "pkg v0.1.0 (/tmp/pkg)";
        let package = package(package_key, false);
        let mut units_by_package = HashMap::new();
        units_by_package.insert(package_key.to_string(), vec![unit_for_package(package_key)]);
        let mut layout_by_package = HashMap::new();
        layout_by_package.insert(
            package_key.to_string(),
            PackageLayoutRequirements {
                target_triples: vec!["x86_64-unknown-linux-gnu".to_string()],
                needs_host_artifacts: true,
            },
        );
        let mut source_prefixes = HashMap::new();
        source_prefixes.insert(package_key.to_string(), vec!["crates/pkg".to_string()]);

        let rendered = build_rendered_package_plans(
            std::slice::from_ref(&package),
            &units_by_package,
            &layout_by_package,
            &source_prefixes,
        );

        assert_eq!(rendered.len(), 1);
        assert_eq!(
            rendered[0].target_triples,
            vec!["x86_64-unknown-linux-gnu".to_string()]
        );
        assert!(rendered[0].needs_host_artifacts);
        assert_eq!(
            rendered[0].workspace_source_prefixes,
            vec!["crates/pkg".to_string()]
        );
        assert!(rendered[0].command_script.contains("run_cargo_cmd"));
    }
}
