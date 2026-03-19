use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use nix_libstore::derivation::Derivation;
use nix_libstore::derived_path::{SingleDerivedPath, SingleDerivedPathBuilt};
use nix_libstore::store_path::StorePath;
use nix_tool::{NixTool, StoreConfig};
use serde::Serialize;
use serde_json::Value;

use crate::command_layout::{package_layout_by_key, PackageLayoutRequirements};
use crate::command_script::render_command_script;
use crate::model::{
    Plan, PlanPackage, Unit, PATH_MARKER_CARGO_BIN, PATH_MARKER_CARGO_HOME,
    PATH_MARKER_RUSTC, PATH_MARKER_SRC, PATH_MARKER_TARGET,
};
use crate::nix_string::{shell_array_literal, shell_single_quote};
use crate::plan_package::{topologically_sorted_packages, units_by_package};
use crate::source_scope::workspace_source_prefixes_by_package;

#[derive(Debug, Clone, Serialize)]
pub struct PackageLayoutInfo {
    pub target_triples: Vec<String>,
    pub needs_host_artifacts: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MaterializedPhaseInfo {
    pub derivation: String,
    pub installable: String,
    pub dependency_phases: BTreeMap<String, DependencyPhase>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackagePhaseInfo {
    pub metadata: Option<MaterializedPhaseInfo>,
    pub full: MaterializedPhaseInfo,
}

#[derive(Clone, Serialize)]
pub struct MaterializedGraph {
    pub manifest_path: String,
    pub workspace_root: String,
    pub target_triple: Option<String>,
    pub workspace_package_keys: Vec<String>,
    pub default_workspace_package_key: Option<String>,
    pub package_names: BTreeMap<String, String>,
    pub package_derivations: BTreeMap<String, String>,
    pub package_installables: BTreeMap<String, String>,
    pub package_phases: BTreeMap<String, PackagePhaseInfo>,
    pub package_layouts: BTreeMap<String, PackageLayoutInfo>,
    #[serde(skip_serializing)]
    package_refs: HashMap<String, SingleDerivedPath>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DependencyPhase {
    Metadata,
    Full,
}

#[derive(Debug, Clone)]
struct PackagePhasePlan {
    metadata_units: Vec<Unit>,
    metadata_dependencies: BTreeMap<String, DependencyPhase>,
    full_units: Vec<Unit>,
    full_dependencies: BTreeMap<String, DependencyPhase>,
}

impl PackagePhasePlan {
    fn has_metadata_phase(&self) -> bool {
        !self.metadata_units.is_empty()
    }
}

impl MaterializedGraph {
    pub fn resolve_target_key(&self, target: &str) -> Result<String> {
        if target == "default" {
            return self
                .default_workspace_package_key
                .clone()
                .ok_or_else(|| anyhow!("no workspace packages available for default target"));
        }

        if self.package_derivations.contains_key(target) {
            return Ok(target.to_string());
        }

        let mut name_matches = self
            .workspace_package_keys
            .iter()
            .filter(|key| self.package_names.get(*key).is_some_and(|name| name == target))
            .cloned()
            .collect::<Vec<_>>();
        name_matches.sort();
        name_matches.dedup();

        match name_matches.len() {
            1 => Ok(name_matches.remove(0)),
            0 => bail!("unknown target `{target}`"),
            _ => bail!("target `{target}` is ambiguous; pass full package key"),
        }
    }

    pub fn build_target(&self, target: &str) -> Result<Vec<String>> {
        let target_key = self.resolve_target_key(target)?;
        let derived_path = self
            .package_refs
            .get(&target_key)
            .cloned()
            .ok_or_else(|| anyhow!("internal error: missing derived path for `{target_key}`"))?;

        let tool = NixTool::new(StoreConfig::default());
        let outputs = tool
            .build(&[derived_path])
            .with_context(|| format!("failed to build target `{target_key}`"))?;

        Ok(outputs.into_iter().map(|path| path.to_string()).collect())
    }
}

pub fn materialize_plan(plan: &Plan, release_mode: bool) -> Result<MaterializedGraph> {
    let ordered_packages = topologically_sorted_packages(plan);
    let units_by_package = units_by_package(plan);
    let package_layout = package_layout_by_key(plan);
    let source_prefixes_by_package = workspace_source_prefixes_by_package(plan);

    let nix_tool = NixTool::new(StoreConfig::default());
    let system = nix_eval_raw("builtins.currentSystem")?;
    let toolchain = Toolchain::resolve()?;

    let needs_cargo_home_snapshot = plan
        .packages
        .iter()
        .any(|package| package.cargo_home_rel_manifest_path.is_some() && !package.workspace_member);
    let cargo_home_snapshot = if needs_cargo_home_snapshot {
        let cargo_home_path = Path::new(&plan.cargo_home);
        Some(
            nix_tool
                .store_add(cargo_home_path)
                .with_context(|| format!("failed to snapshot cargo home {}", cargo_home_path.display()))?,
        )
    } else {
        None
    };

    let workspace_root = Path::new(&plan.workspace_root);
    let mut package_names = BTreeMap::new();
    let mut package_derivations = BTreeMap::new();
    let mut package_installables = BTreeMap::new();
    let mut package_phases = BTreeMap::new();
    let mut package_layouts = BTreeMap::new();
    let mut package_refs: HashMap<String, SingleDerivedPath> = HashMap::new();
    let mut package_output_paths: HashMap<String, String> = HashMap::new();
    let mut package_metadata_refs: HashMap<String, SingleDerivedPath> = HashMap::new();
    let mut package_metadata_output_paths: HashMap<String, String> = HashMap::new();

    for (package_index, package) in ordered_packages.iter().enumerate() {
        let layout = package_layout
            .get(package.key.as_str())
            .cloned()
            .unwrap_or(PackageLayoutRequirements {
                target_triples: Vec::new(),
                needs_host_artifacts: false,
            });
        let units = units_by_package
            .get(package.key.as_str())
            .cloned()
            .unwrap_or_default();
        let phase_plan = build_package_phase_plan(plan, package, &units);
        let source_prefixes = source_prefixes_by_package
            .get(package.key.as_str())
            .cloned()
            .unwrap_or_default();

        let staged_source = stage_package_source(workspace_root, &source_prefixes)
            .with_context(|| format!("failed to stage package source for {}", package.key))?;
        let source_store_name = format!(
            "nix-cargo-src-{}-{}",
            sanitize_derivation_component(&package.name),
            package_index
        );
        let source_store_path = nix_tool
            .store_add_named(&staged_source, Some(&source_store_name))
            .with_context(|| format!("failed to add staged source for {}", package.key))?;
        let _ = fs::remove_dir_all(&staged_source);

        let mut metadata_phase_info = None;
        if phase_plan.has_metadata_phase() {
            let metadata_dependency_refs = resolve_phase_dependency_refs(
                &phase_plan.metadata_dependencies,
                &package_refs,
                &package_metadata_refs,
            )?;
            let metadata_dependency_output_paths = resolve_phase_dependency_output_paths(
                &phase_plan.metadata_dependencies,
                &package_output_paths,
                &package_metadata_output_paths,
            )?;

            let (metadata_drv_path, metadata_ref, metadata_output_path) =
                materialize_package_derivation_phase(
                    plan,
                    package,
                    package_index,
                    Some("metadata"),
                    &phase_plan.metadata_units,
                    &layout,
                    &toolchain,
                    &nix_tool,
                    &system,
                    &source_store_path,
                    cargo_home_snapshot.as_ref(),
                    &metadata_dependency_refs,
                    &metadata_dependency_output_paths,
                    release_mode,
                )
                .with_context(|| {
                    format!("failed to materialize metadata phase for {}", package.key)
                })?;

            metadata_phase_info = Some(MaterializedPhaseInfo {
                derivation: metadata_drv_path.to_string(),
                installable: metadata_ref.to_string(),
                dependency_phases: phase_plan.metadata_dependencies.clone(),
            });
            package_metadata_refs.insert(package.key.clone(), metadata_ref);
            package_metadata_output_paths.insert(package.key.clone(), metadata_output_path);
        }

        let mut full_dependency_refs = resolve_phase_dependency_refs(
            &phase_plan.full_dependencies,
            &package_refs,
            &package_metadata_refs,
        )?;
        let full_dependency_output_paths = resolve_phase_dependency_output_paths(
            &phase_plan.full_dependencies,
            &package_output_paths,
            &package_metadata_output_paths,
        )?;
        if let Some(metadata_ref) = package_metadata_refs.get(&package.key).cloned() {
            full_dependency_refs.push(metadata_ref);
        }

        let (drv_path, package_ref, output_path) = materialize_package_derivation_phase(
            plan,
            package,
            package_index,
            None,
            &phase_plan.full_units,
            &layout,
            &toolchain,
            &nix_tool,
            &system,
            &source_store_path,
            cargo_home_snapshot.as_ref(),
            &full_dependency_refs,
            &full_dependency_output_paths,
            release_mode,
        )
        .with_context(|| format!("failed to materialize full phase for {}", package.key))?;

        package_names.insert(package.key.clone(), package.name.clone());
        package_derivations.insert(package.key.clone(), drv_path.to_string());
        package_installables.insert(package.key.clone(), package_ref.to_string());
        package_phases.insert(
            package.key.clone(),
            PackagePhaseInfo {
                metadata: metadata_phase_info,
                full: MaterializedPhaseInfo {
                    derivation: drv_path.to_string(),
                    installable: package_ref.to_string(),
                    dependency_phases: phase_plan.full_dependencies.clone(),
                },
            },
        );
        package_layouts.insert(
            package.key.clone(),
            PackageLayoutInfo {
                target_triples: layout.target_triples.clone(),
                needs_host_artifacts: layout.needs_host_artifacts,
            },
        );
        package_refs.insert(package.key.clone(), package_ref);
        package_output_paths.insert(package.key.clone(), output_path);
    }

    let workspace_package_keys = plan
        .packages
        .iter()
        .filter(|package| package.workspace_member)
        .map(|package| package.key.clone())
        .collect::<Vec<_>>();
    let default_workspace_package_key = workspace_package_keys.first().cloned();

    Ok(MaterializedGraph {
        manifest_path: plan.manifest_path.clone(),
        workspace_root: plan.workspace_root.clone(),
        target_triple: plan.target_triple.clone(),
        workspace_package_keys,
        default_workspace_package_key,
        package_names,
        package_derivations,
        package_installables,
        package_phases,
        package_layouts,
        package_refs,
    })
}

fn materialize_package_derivation_phase(
    plan: &Plan,
    package: &PlanPackage,
    package_index: usize,
    phase_suffix: Option<&str>,
    units: &[Unit],
    layout: &PackageLayoutRequirements,
    toolchain: &Toolchain,
    nix_tool: &NixTool,
    system: &str,
    source_store_path: &StorePath,
    cargo_home_snapshot: Option<&StorePath>,
    dependency_refs: &[SingleDerivedPath],
    dependency_output_paths: &[String],
    release_mode: bool,
) -> Result<(StorePath, SingleDerivedPath, String)> {
    let command_script = render_command_script(units);
    let build_script = render_package_builder_script(
        plan,
        package,
        layout,
        toolchain,
        source_store_path,
        cargo_home_snapshot,
        dependency_output_paths,
        &command_script,
        release_mode,
    );

    let drv_name = match phase_suffix {
        Some(suffix) => format!(
            "nix-cargo-{}-{}-{suffix}",
            sanitize_derivation_component(&package.name),
            package_index
        ),
        None => format!(
            "nix-cargo-{}-{}",
            sanitize_derivation_component(&package.name),
            package_index
        ),
    };
    let mut derivation = Derivation::new(&drv_name, system, &toolchain.bash_builder());
    derivation
        .add_arg("-euo")
        .add_arg("pipefail")
        .add_arg("-c")
        .add_arg(&build_script)
        .set_env("PATH", &toolchain.path_env())
        .add_output("out", None, None, None);
    derivation.add_input_src(source_store_path);
    if let Some(cargo_home_store) = cargo_home_snapshot {
        derivation.add_input_src(cargo_home_store);
    }
    for dep in dependency_refs {
        derivation.add_derived_path(dep);
    }
    toolchain.add_inputs(&mut derivation);

    let drv_path = nix_tool.derivation_add(&derivation)?;
    let package_ref =
        SingleDerivedPath::Built(SingleDerivedPathBuilt::new(drv_path.clone(), "out".to_string()));
    let output_path = derivation_output_path(nix_tool, &drv_path, "out")?;

    Ok((drv_path, package_ref, output_path))
}

fn build_package_phase_plan(plan: &Plan, package: &PlanPackage, units: &[Unit]) -> PackagePhasePlan {
    let has_metadata_phase = units.iter().any(is_metadata_phase_unit);
    let metadata_units = if has_metadata_phase {
        units.iter()
            .filter(|unit| is_build_script_compile_unit(unit) || is_metadata_phase_unit(unit))
            .cloned()
            .map(rewrite_unit_for_metadata_phase)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let full_units = units.to_vec();

    let metadata_dependencies = dependency_phase_requirements(plan, package, &metadata_units);
    let full_dependencies = dependency_phase_requirements(plan, package, &full_units);

    PackagePhasePlan {
        metadata_units,
        metadata_dependencies,
        full_units,
        full_dependencies,
    }
}

fn resolve_phase_dependency_refs(
    requirements: &BTreeMap<String, DependencyPhase>,
    full_refs: &HashMap<String, SingleDerivedPath>,
    metadata_refs: &HashMap<String, SingleDerivedPath>,
) -> Result<Vec<SingleDerivedPath>> {
    let mut refs = Vec::with_capacity(requirements.len());
    for (package_key, phase) in requirements {
        let reference = match phase {
            DependencyPhase::Metadata => metadata_refs
                .get(package_key)
                .or_else(|| full_refs.get(package_key))
                .cloned(),
            DependencyPhase::Full => full_refs.get(package_key).cloned(),
        }
        .ok_or_else(|| anyhow!("missing {:?} dependency reference for `{package_key}`", phase))?;
        refs.push(reference);
    }
    Ok(refs)
}

fn resolve_phase_dependency_output_paths(
    requirements: &BTreeMap<String, DependencyPhase>,
    full_output_paths: &HashMap<String, String>,
    metadata_output_paths: &HashMap<String, String>,
) -> Result<Vec<String>> {
    let mut output_paths = Vec::with_capacity(requirements.len());
    for (package_key, phase) in requirements {
        let output_path = match phase {
            DependencyPhase::Metadata => metadata_output_paths
                .get(package_key)
                .or_else(|| full_output_paths.get(package_key))
                .cloned(),
            DependencyPhase::Full => full_output_paths.get(package_key).cloned(),
        }
        .ok_or_else(|| anyhow!("missing {:?} dependency output path for `{package_key}`", phase))?;
        output_paths.push(output_path);
    }
    Ok(output_paths)
}

fn dependency_phase_requirements(
    plan: &Plan,
    package: &PlanPackage,
    units: &[Unit],
) -> BTreeMap<String, DependencyPhase> {
    let dependency_keys = package
        .dependencies
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    let mut requirements = BTreeMap::new();
    for unit in units {
        for (dependency_key, phase) in unit_dependency_requirements(plan, &dependency_keys, unit) {
            requirements
                .entry(dependency_key)
                .and_modify(|current| *current = stronger_phase(*current, phase))
                .or_insert(phase);
        }
    }
    requirements
}

fn unit_dependency_requirements(
    plan: &Plan,
    dependency_keys: &[String],
    unit: &Unit,
) -> Vec<(String, DependencyPhase)> {
    let mut requirements = Vec::new();
    let dependency_name_by_key = dependency_keys
        .iter()
        .filter_map(|key| {
            plan.packages
                .iter()
                .find(|package| package.key == *key)
                .map(|package| (key.clone(), normalize_crate_name(&package.name)))
        })
        .collect::<Vec<_>>();

    for (crate_name, artifact_path) in command_extern_artifacts(&unit.command) {
        if artifact_path.is_empty() {
            continue;
        }

        let mut candidate_names = Vec::new();
        if !crate_name.is_empty() {
            candidate_names.push(normalize_crate_name(&crate_name));
        }
        if let Some(artifact_stem) = artifact_crate_stem(&artifact_path) {
            let normalized = normalize_crate_name(&artifact_stem);
            if !candidate_names.iter().any(|candidate| candidate == &normalized) {
                candidate_names.push(normalized);
            }
        }

        if candidate_names.is_empty() {
            continue;
        }

        let phase = dependency_phase_for_artifact(&artifact_path);
        for (dependency_key, dependency_name) in &dependency_name_by_key {
            if candidate_names.iter().any(|candidate| candidate == dependency_name) {
                requirements.push((dependency_key.clone(), phase));
            }
        }
    }

    requirements
}

fn stronger_phase(current: DependencyPhase, next: DependencyPhase) -> DependencyPhase {
    match (current, next) {
        (DependencyPhase::Full, _) | (_, DependencyPhase::Full) => DependencyPhase::Full,
        _ => DependencyPhase::Metadata,
    }
}

fn is_build_script_compile_unit(unit: &Unit) -> bool {
    (unit.target_kind == "build-script" || unit.target_kind == "custom-build")
        && unit.compile_mode == "Build"
}

fn is_metadata_phase_unit(unit: &Unit) -> bool {
    command_emits_metadata(&unit.command) && command_has_lib_crate_type(&unit.command)
}

fn rewrite_unit_for_metadata_phase(unit: Unit) -> Unit {
    if !is_metadata_phase_unit(&unit) {
        return unit;
    }

    let mut rewritten = unit;
    rewritten.command = rewrite_command_for_metadata_phase(rewritten.command);
    rewritten
}

fn rewrite_command_for_metadata_phase(mut command: crate::model::CommandSpec) -> crate::model::CommandSpec {
    let mut rewritten_args = Vec::with_capacity(command.args.len());
    let mut args = command.args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--emit" {
            rewritten_args.push(arg);
            if let Some(value) = args.next() {
                rewritten_args.push(metadata_emit_value(&value));
            }
            continue;
        }

        if let Some(value) = arg.strip_prefix("--emit=") {
            rewritten_args.push(format!("--emit={}", metadata_emit_value(value)));
            continue;
        }

        rewritten_args.push(arg);
    }
    command.args = rewritten_args;
    command
}

fn metadata_emit_value(value: &str) -> String {
    let mut kept = Vec::new();
    for part in value.split(',') {
        if part == "metadata"
            || part == "dep-info"
            || part.starts_with("dep-info=")
            || part.starts_with("metadata=")
        {
            kept.push(part.to_string());
        }
    }
    if !kept.iter().any(|part| part == "metadata") {
        kept.push("metadata".to_string());
    }
    kept.join(",")
}

fn command_emits_metadata(command: &crate::model::CommandSpec) -> bool {
    for arg in &command.args {
        if let Some(value) = arg.strip_prefix("--emit=") {
            return value.split(',').any(|part| part == "metadata");
        }
    }

    let mut args = command.args.iter();
    while let Some(arg) = args.next() {
        if arg == "--emit" {
            return args
                .next()
                .is_some_and(|value| value.split(',').any(|part| part == "metadata"));
        }
    }

    false
}

fn command_has_lib_crate_type(command: &crate::model::CommandSpec) -> bool {
    for arg in &command.args {
        if let Some(value) = arg.strip_prefix("--crate-type=") {
            return value == "lib";
        }
    }

    let mut args = command.args.iter();
    while let Some(arg) = args.next() {
        if arg == "--crate-type" {
            return args.next().is_some_and(|value| value == "lib");
        }
    }

    false
}

fn command_extern_artifacts(command: &crate::model::CommandSpec) -> Vec<(String, String)> {
    let mut artifacts = Vec::new();
    let mut args = command.args.iter();
    while let Some(arg) = args.next() {
        if arg == "--extern" {
            if let Some(spec) = args.next() {
                if let Some((crate_name, path)) = parse_extern_spec(spec) {
                    artifacts.push((crate_name, path));
                }
            }
            continue;
        }

        if let Some(spec) = arg.strip_prefix("--extern=") {
            if let Some((crate_name, path)) = parse_extern_spec(spec) {
                artifacts.push((crate_name, path));
            }
        }
    }
    artifacts
}

fn parse_extern_spec(spec: &str) -> Option<(String, String)> {
    let (crate_name, artifact_path) = spec.split_once('=')?;
    Some((crate_name.to_string(), artifact_path.to_string()))
}

fn dependency_phase_for_artifact(artifact_path: &str) -> DependencyPhase {
    if artifact_path.ends_with(".rmeta") {
        DependencyPhase::Metadata
    } else {
        DependencyPhase::Full
    }
}

fn artifact_crate_stem(artifact_path: &str) -> Option<String> {
    let file_name = Path::new(artifact_path).file_name()?.to_str()?;
    let without_ext = file_name
        .trim_end_matches(".rmeta")
        .trim_end_matches(".rlib")
        .trim_end_matches(".so")
        .trim_end_matches(".dylib")
        .trim_end_matches(".dll")
        .trim_end_matches(".a");
    let without_prefix = without_ext.strip_prefix("lib").unwrap_or(without_ext);
    let (crate_stem, hash_suffix) = without_prefix.rsplit_once('-')?;
    if hash_suffix.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Some(crate_stem.to_string())
    } else {
        Some(without_prefix.to_string())
    }
}

fn normalize_crate_name(value: &str) -> String {
    value.replace('-', "_")
}

struct Toolchain {
    bash: NixpkgsTool,
    coreutils: NixpkgsTool,
    cargo: NixpkgsTool,
    rustc: NixpkgsTool,
    pkg_config: NixpkgsTool,
    cc: NixpkgsTool,
}

struct NixpkgsTool {
    drv_path: StorePath,
    out_path: StorePath,
}

impl Toolchain {
    fn resolve() -> Result<Self> {
        Ok(Self {
            bash: resolve_tool("BASH", "bash")?,
            coreutils: resolve_tool("COREUTILS", "coreutils")?,
            cargo: resolve_tool("CARGO", "cargo")?,
            rustc: resolve_tool("RUSTC", "rustc")?,
            pkg_config: resolve_tool("PKG_CONFIG", "pkg-config")?,
            cc: resolve_tool("CC", "stdenv.cc")?,
        })
    }

    fn bash_builder(&self) -> String {
        format!("{}/bin/bash", self.bash.out_path)
    }

    fn rustc_bin(&self) -> String {
        format!("{}/bin/rustc", self.rustc.out_path)
    }

    fn cargo_bin(&self) -> String {
        format!("{}/bin/cargo", self.cargo.out_path)
    }

    fn path_env(&self) -> String {
        format!(
            "{}/bin:{}/bin:{}/bin:{}/bin",
            self.coreutils.out_path, self.pkg_config.out_path, self.cc.out_path, self.bash.out_path
        )
    }

    fn add_inputs(&self, derivation: &mut Derivation) {
        for tool in [
            &self.bash,
            &self.coreutils,
            &self.cargo,
            &self.rustc,
            &self.pkg_config,
            &self.cc,
        ] {
            let reference =
                SingleDerivedPath::Built(SingleDerivedPathBuilt::new(tool.drv_path.clone(), "out".to_string()));
            derivation.add_derived_path(&reference);
        }
    }
}

fn resolve_tool(env_prefix: &str, attr: &str) -> Result<NixpkgsTool> {
    resolve_tool_from_env(env_prefix)?.map_or_else(|| resolve_nixpkgs_tool(attr), Ok)
}

fn resolve_tool_from_env(env_prefix: &str) -> Result<Option<NixpkgsTool>> {
    let drv_key = format!("NIXCARGO_TOOL_{env_prefix}_DRV");
    let out_key = format!("NIXCARGO_TOOL_{env_prefix}_OUT");
    match (env::var(&drv_key).ok(), env::var(&out_key).ok()) {
        (None, None) => Ok(None),
        (Some(drv_path), Some(out_path)) => Ok(Some(NixpkgsTool {
            drv_path: StorePath::new(drv_path)
                .with_context(|| format!("invalid store path in environment variable `{drv_key}`"))?,
            out_path: StorePath::new(out_path)
                .with_context(|| format!("invalid store path in environment variable `{out_key}`"))?,
        })),
        _ => bail!(
            "expected both `{drv_key}` and `{out_key}` to be set when overriding tool resolution"
        ),
    }
}

fn resolve_nixpkgs_tool(attr: &str) -> Result<NixpkgsTool> {
    let drv_installable = format!("nixpkgs#{attr}.drvPath");
    let out_installable = format!("nixpkgs#{attr}.outPath");
    let drv_path = StorePath::new(nix_eval_installable_raw(&drv_installable)?)
        .with_context(|| format!("invalid drvPath for nixpkgs attr `{attr}`"))?;
    let out_path = StorePath::new(nix_eval_installable_raw(&out_installable)?)
        .with_context(|| format!("invalid outPath for nixpkgs attr `{attr}`"))?;
    Ok(NixpkgsTool { drv_path, out_path })
}

fn nix_eval_raw(expr: &str) -> Result<String> {
    let output = Command::new("nix")
        .args(["eval", "--impure", "--raw", "--expr", expr])
        .output()
        .with_context(|| format!("failed to run `nix eval` for `{expr}`"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("`nix eval` failed for `{expr}`: {stderr}");
    }

    Ok(String::from_utf8(output.stdout)
        .context("failed to decode `nix eval` output")?
        .trim()
        .to_string())
}

fn nix_eval_installable_raw(installable: &str) -> Result<String> {
    let output = Command::new("nix")
        .args(["eval", "--raw", installable])
        .output()
        .with_context(|| format!("failed to run `nix eval` for `{installable}`"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("`nix eval` failed for `{installable}`: {stderr}");
    }

    Ok(String::from_utf8(output.stdout)
        .context("failed to decode `nix eval` output")?
        .trim()
        .to_string())
}

fn derivation_output_path(nix_tool: &NixTool, drv_path: &StorePath, output_name: &str) -> Result<String> {
    let shown = nix_tool.derivation_show(drv_path)?;
    let json: Value = serde_json::from_slice(&shown.stdout)
        .with_context(|| format!("failed to parse derivation show JSON for {}", drv_path))?;

    let drv_path_string = drv_path.to_string();
    let drv_basename = Path::new(&drv_path_string)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("invalid derivation path {}", drv_path))?;
    let output_basename = json
        .get("derivations")
        .and_then(|derivations| derivations.get(drv_basename))
        .and_then(|drv| drv.get("outputs"))
        .and_then(|outputs| outputs.get(output_name))
        .and_then(|output| output.get("path"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing output `{output_name}` in derivation show for {}", drv_path))?;

    let store_dir = Path::new(&drv_path_string)
        .parent()
        .ok_or_else(|| anyhow!("failed to determine store dir from {}", drv_path))?;
    Ok(store_dir.join(output_basename).display().to_string())
}

fn stage_package_source(workspace_root: &Path, prefixes: &[String]) -> Result<PathBuf> {
    let staging_root = create_temp_dir("nix-cargo-src")?;
    copy_if_exists(
        &workspace_root.join("Cargo.toml"),
        &staging_root.join("Cargo.toml"),
    )?;
    copy_if_exists(
        &workspace_root.join("Cargo.lock"),
        &staging_root.join("Cargo.lock"),
    )?;
    copy_if_exists(&workspace_root.join(".cargo"), &staging_root.join(".cargo"))?;

    for prefix in prefixes {
        let src = workspace_root.join(prefix);
        let dst = staging_root.join(prefix);
        copy_if_exists(&src, &dst)?;
    }

    Ok(staging_root)
}

fn create_temp_dir(prefix: &str) -> Result<PathBuf> {
    let base = std::env::temp_dir();
    let pid = std::process::id();
    for attempt in 0..2048u32 {
        let candidate = base.join(format!("{prefix}-{pid}-{attempt}"));
        match fs::create_dir(&candidate) {
            Ok(_) => return Ok(candidate),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("failed to create temp dir {}", candidate.display())
                });
            }
        }
    }
    bail!("failed to allocate temp dir for prefix `{prefix}`");
}

fn copy_if_exists(src: &Path, dst: &Path) -> Result<()> {
    if !src.exists() {
        return Ok(());
    }
    copy_path_recursive(src, dst)
}

fn copy_path_recursive(src: &Path, dst: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(src)
        .with_context(|| format!("failed to stat {}", src.display()))?;

    if metadata.file_type().is_symlink() {
        copy_symlink(src, dst)?;
        return Ok(());
    }

    if metadata.is_dir() {
        fs::create_dir_all(dst)
            .with_context(|| format!("failed to create directory {}", dst.display()))?;
        for entry in fs::read_dir(src).with_context(|| format!("failed to read {}", src.display()))? {
            let entry = entry.with_context(|| format!("failed to read entry under {}", src.display()))?;
            let child_src = entry.path();
            let child_dst = dst.join(entry.file_name());
            copy_path_recursive(&child_src, &child_dst)?;
        }
        return Ok(());
    }

    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent {}", parent.display()))?;
    }
    fs::copy(src, dst)
        .with_context(|| format!("failed to copy {} -> {}", src.display(), dst.display()))?;
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(src: &Path, dst: &Path) -> Result<()> {
    use std::os::unix::fs as unix_fs;

    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create parent {}", parent.display()))?;
    }
    let target = fs::read_link(src)
        .with_context(|| format!("failed to read symlink {}", src.display()))?;
    unix_fs::symlink(&target, dst)
        .with_context(|| format!("failed to write symlink {} -> {}", dst.display(), target.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn copy_symlink(src: &Path, dst: &Path) -> Result<()> {
    let canonical = fs::canonicalize(src)
        .with_context(|| format!("failed to canonicalize symlink {}", src.display()))?;
    copy_path_recursive(&canonical, dst)
}

fn sanitize_derivation_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').to_string()
}

fn render_package_builder_script(
    plan: &Plan,
    _package: &PlanPackage,
    layout: &PackageLayoutRequirements,
    toolchain: &Toolchain,
    source_store_path: &StorePath,
    cargo_home_snapshot: Option<&StorePath>,
    dependency_output_paths: &[String],
    command_script: &str,
    release_mode: bool,
) -> String {
    let build_mode = if release_mode { "release" } else { "debug" };
    let dep_paths = dependency_output_paths.to_vec();
    let target_triples = layout.target_triples.clone();
    let mut script = String::new();

    script.push_str("set -euo pipefail\n");
    script.push_str("export CARGO_TARGET_DIR=\"$TMPDIR/target\"\n");
    script.push_str("mkdir -p \"$CARGO_TARGET_DIR\"\n");
    script.push_str("mkdir -p \"$TMPDIR/home\"\n");
    script.push_str("export HOME=\"$TMPDIR/home\"\n");
    script.push_str(&format!(
        "export NIXCARGO_SRC={}\n",
        shell_single_quote(&source_store_path.to_string())
    ));
    if let Some(cargo_home) = cargo_home_snapshot {
        script.push_str(&format!(
            "export NIXCARGO_CARGO_HOME={}\n",
            shell_single_quote(&cargo_home.to_string())
        ));
    } else {
        script.push_str("export NIXCARGO_CARGO_HOME=\"$TMPDIR/cargo-home\"\n");
        script.push_str("mkdir -p \"$NIXCARGO_CARGO_HOME\"\n");
    }
    script.push_str(&format!(
        "export NIXCARGO_RUSTC={}\n",
        shell_single_quote(&toolchain.rustc_bin())
    ));
    script.push_str(&format!(
        "export NIXCARGO_CARGO={}\n",
        shell_single_quote(&toolchain.cargo_bin())
    ));
    script.push_str("export CARGO_HOME=\"$NIXCARGO_CARGO_HOME\"\n");
    script.push_str(&format!(
        "markerTarget={}\n",
        shell_single_quote(PATH_MARKER_TARGET)
    ));
    script.push_str(&format!("markerSrc={}\n", shell_single_quote(PATH_MARKER_SRC)));
    script.push_str(&format!(
        "markerCargoHome={}\n",
        shell_single_quote(PATH_MARKER_CARGO_HOME)
    ));
    script.push_str(&format!(
        "markerRustc={}\n",
        shell_single_quote(PATH_MARKER_RUSTC)
    ));
    script.push_str(&format!(
        "markerCargoBin={}\n",
        shell_single_quote(PATH_MARKER_CARGO_BIN)
    ));
    script.push_str(&format!(
        "planWorkspaceRoot={}\n",
        shell_single_quote(&plan.workspace_root)
    ));
    script.push_str(&format!(
        "planTargetDir={}\n",
        shell_single_quote(&plan.target_dir)
    ));
    script.push_str(&format!("buildMode={}\n", shell_single_quote(build_mode)));
    script.push_str(&format!(
        "depPaths=({})\n",
        shell_array_literal(&dep_paths)
    ));
    script.push_str(&format!(
        "targetTriples=({})\n",
        shell_array_literal(&target_triples)
    ));
    script.push_str(&format!(
        "needsHostArtifacts={}\n",
        if layout.needs_host_artifacts { "1" } else { "0" }
    ));
    script.push_str("declare -A nixcargo_build_script_runs=()\n");
    script.push_str("declare -A nixcargo_build_script_binaries=()\n");
    script.push_str("mkdir -p \"$CARGO_TARGET_DIR/$buildMode/deps\"\n");
    script.push_str("mkdir -p \"$CARGO_TARGET_DIR/$buildMode/build\"\n");
    script.push_str("mkdir -p \"$CARGO_TARGET_DIR/$buildMode/examples\"\n");
    script.push_str("for targetTriple in \"${targetTriples[@]}\"; do\n");
    script.push_str("  mkdir -p \"$CARGO_TARGET_DIR/$targetTriple/$buildMode/deps\"\n");
    script.push_str("  mkdir -p \"$CARGO_TARGET_DIR/$targetTriple/$buildMode/build\"\n");
    script.push_str("  mkdir -p \"$CARGO_TARGET_DIR/$targetTriple/$buildMode/examples\"\n");
    script.push_str("done\n");
    script.push_str(
        r#"
copy_tree_if_exists() {
  local srcDir="$1"
  local dstDir="$2"
  if [ -d "$srcDir" ]; then
    mkdir -p "$dstDir"
    cp -R -n "$srcDir/." "$dstDir/"
  fi
}
for depPath in "${depPaths[@]}"; do
  if [ -d "$depPath" ]; then
    if [ "$needsHostArtifacts" -eq 1 ]; then
      copy_tree_if_exists "$depPath/deps" "$CARGO_TARGET_DIR/$buildMode/deps"
      copy_tree_if_exists "$depPath/build" "$CARGO_TARGET_DIR/$buildMode/build"
      copy_tree_if_exists "$depPath/examples" "$CARGO_TARGET_DIR/$buildMode/examples"
      copy_tree_if_exists "$depPath/.fingerprint" "$CARGO_TARGET_DIR/$buildMode/.fingerprint"
    fi
    for targetTriple in "${targetTriples[@]}"; do
      copy_tree_if_exists "$depPath/$targetTriple/deps" "$CARGO_TARGET_DIR/$targetTriple/$buildMode/deps"
      copy_tree_if_exists "$depPath/$targetTriple/build" "$CARGO_TARGET_DIR/$targetTriple/$buildMode/build"
      copy_tree_if_exists "$depPath/$targetTriple/examples" "$CARGO_TARGET_DIR/$targetTriple/$buildMode/examples"
      copy_tree_if_exists "$depPath/$targetTriple/.fingerprint" "$CARGO_TARGET_DIR/$targetTriple/$buildMode/.fingerprint"
    done
  fi
done
rewrite_value() {
  local value="$1"
  value="${value//${markerTarget}/$CARGO_TARGET_DIR}"
  value="${value//${markerSrc}/$NIXCARGO_SRC}"
  value="${value//${markerCargoHome}/$NIXCARGO_CARGO_HOME}"
  value="${value//${markerRustc}/$NIXCARGO_RUSTC}"
  value="${value//${markerCargoBin}/$NIXCARGO_CARGO}"
  value="${value//${planWorkspaceRoot}/$NIXCARGO_SRC}"
  value="${value//${planTargetDir}/$CARGO_TARGET_DIR}"
  printf '%s' "$value"
}
run_cargo_cmd() {
  local isBuildScriptCompile="$1"
  shift
  local buildScriptBinaryHintRaw="$1"
  shift
  local cwdRaw="$1"
  shift
  local programRaw="$1"
  shift
  local -n argsRef="$1"
  shift
  local -n envRef="$1"
  local cwd program entry key value status outDir manifestDir runDir outputPath buildScriptBinary buildScriptBinaryHint
  local -a args=()
  local -a envArgs=()
  cwd="$(rewrite_value "$cwdRaw")"
  program="$(rewrite_value "$programRaw")"
  buildScriptBinaryHint="$(rewrite_value "$buildScriptBinaryHintRaw")"
  for entry in "${envRef[@]}"; do
    key="${entry%%=*}"
    value="${entry#*=}"
    envArgs+=("${key}=$(rewrite_value "$value")")
  done
  local -a rewrittenArgs=()
  local nextIndex nextValue scanIndex emitEntry
  for entry in "${argsRef[@]}"; do
    rewrittenArgs+=("$(rewrite_value "$entry")")
  done
  ensure_parent_dir() {
    local path="$1"
    if [ -n "$path" ]; then
      mkdir -p "$(dirname "$path")"
    fi
  }
  ensure_dep_info_parent_dirs() {
    local emitSpecValue="$1"
    local -a emitParts=()
    IFS=',' read -r -a emitParts <<< "$emitSpecValue"
    for emitEntry in "${emitParts[@]}"; do
      if [[ "$emitEntry" == dep-info=* ]]; then
        ensure_parent_dir "${emitEntry#dep-info=}"
      fi
    done
  }
  scanIndex=0
  while [ "$scanIndex" -lt "${#rewrittenArgs[@]}" ]; do
    entry="${rewrittenArgs[$scanIndex]}"
    if [ "$entry" = "--out-dir" ] && [ $((scanIndex + 1)) -lt "${#rewrittenArgs[@]}" ]; then
      mkdir -p "${rewrittenArgs[$((scanIndex + 1))]}"
    elif [[ "$entry" == --out-dir=* ]]; then
      mkdir -p "${entry#--out-dir=}"
    elif [ "$entry" = "-o" ] && [ $((scanIndex + 1)) -lt "${#rewrittenArgs[@]}" ]; then
      ensure_parent_dir "${rewrittenArgs[$((scanIndex + 1))]}"
    elif [ "$entry" = "--emit" ] && [ $((scanIndex + 1)) -lt "${#rewrittenArgs[@]}" ]; then
      ensure_dep_info_parent_dirs "${rewrittenArgs[$((scanIndex + 1))]}"
    elif [[ "$entry" == --emit=* ]]; then
      ensure_dep_info_parent_dirs "${entry#--emit=}"
    fi
    scanIndex=$((scanIndex + 1))
  done
  nextIndex=0
  while [ "$nextIndex" -lt "${#rewrittenArgs[@]}" ]; do
    entry="${rewrittenArgs[$nextIndex]}"
    if [ "$entry" = "-C" ] && [ $((nextIndex + 1)) -lt "${#rewrittenArgs[@]}" ]; then
      nextValue="${rewrittenArgs[$((nextIndex + 1))]}"
      if [[ "$nextValue" == incremental=* ]]; then
        nextIndex=$((nextIndex + 2))
        continue
      fi
    fi
    if [[ "$entry" == -Cincremental=* ]]; then
      nextIndex=$((nextIndex + 1))
      continue
    fi
    args+=("$entry")
    nextIndex=$((nextIndex + 1))
  done
  outDir=""
  manifestDir=""
  for entry in "${envArgs[@]}"; do
    key="${entry%%=*}"
    value="${entry#*=}"
    if [ "$key" = "OUT_DIR" ]; then
      outDir="$value"
    fi
    if [ "$key" = "CARGO_MANIFEST_DIR" ]; then
      manifestDir="$value"
    fi
  done
  runDir="$manifestDir"
  if [ -z "$runDir" ]; then
    runDir="$cwd"
  fi
  outputPath=""
  nextIndex=0
  while [ "$nextIndex" -lt "${#args[@]}" ]; do
    entry="${args[$nextIndex]}"
    if [ "$entry" = "-o" ] && [ $((nextIndex + 1)) -lt "${#args[@]}" ]; then
      outputPath="${args[$((nextIndex + 1))]}"
      nextIndex=$((nextIndex + 2))
      continue
    fi
    nextIndex=$((nextIndex + 1))
  done
  buildScriptBinary=""
  if [ -n "$buildScriptBinaryHint" ]; then
    buildScriptBinary="$buildScriptBinaryHint"
  elif [ -n "$runDir" ] && [ -n "${nixcargo_build_script_binaries[$runDir]:-}" ]; then
    buildScriptBinary="${nixcargo_build_script_binaries[$runDir]}"
  fi
  if [ "$isBuildScriptCompile" -ne 1 ] && [ -n "$outDir" ] && [ -n "$buildScriptBinary" ] && [ -x "$buildScriptBinary" ] && [ -z "${nixcargo_build_script_runs[$outDir]+x}" ]; then
    mkdir -p "$outDir"
    if [ -n "$runDir" ]; then
      (cd "$runDir" && env "${envArgs[@]}" OUT_DIR="$outDir" CARGO_MANIFEST_DIR="$runDir" "$buildScriptBinary")
    else
      env "${envArgs[@]}" OUT_DIR="$outDir" CARGO_MANIFEST_DIR="$runDir" "$buildScriptBinary"
    fi
    nixcargo_build_script_runs[$outDir]=1
  fi
  if [ -n "$cwd" ]; then
    pushd "$cwd" > /dev/null
  fi
  env "${envArgs[@]}" "$program" "${args[@]}"
  status=$?
  if [ "$status" -eq 0 ] && [ "$isBuildScriptCompile" -eq 1 ]; then
    if [ -z "$outputPath" ] && [ -n "$buildScriptBinaryHint" ]; then
      outputPath="$buildScriptBinaryHint"
    fi
    if [ -z "$outputPath" ]; then
      echo "nix-cargo: missing build-script binary path for runDir=$runDir" >&2
      return 1
    fi
    if [ -n "$outputPath" ] && [ -x "$outputPath" ]; then
      if [ -n "$runDir" ]; then
        nixcargo_build_script_binaries[$runDir]="$outputPath"
      fi
    fi
  fi
  if [ -n "$cwd" ]; then
    popd > /dev/null
  fi
  return "$status"
}
"#,
    );

    script.push_str("scriptFile=\"$TMPDIR/nix-cargo-package-commands.sh\"\n");
    script.push_str("cat > \"$scriptFile\" <<'__NIX_CARGO_COMMANDS__'\n");
    script.push_str(command_script);
    if !command_script.ends_with('\n') {
        script.push('\n');
    }
    script.push_str("__NIX_CARGO_COMMANDS__\n");
    script.push_str("source \"$scriptFile\"\n");
    script.push_str(
        r#"
mkdir -p "$out"
copied=0
copy_install_layout() {
  local srcRoot="$1"
  local dstRoot="$2"
  if [ -d "$srcRoot/deps" ]; then
    mkdir -p "$dstRoot/deps"
    shopt -s nullglob
    for artifact in "$srcRoot/deps"/*; do
      case "$artifact" in
        *.d) continue ;;
      esac
      cp -R "$artifact" "$dstRoot/deps/"
    done
    shopt -u nullglob
    copied=1
  fi
  if [ -d "$srcRoot/build" ]; then
    cp -R "$srcRoot/build" "$dstRoot/"
    copied=1
  fi
  if [ -d "$srcRoot/examples" ]; then
    cp -R "$srcRoot/examples" "$dstRoot/"
    copied=1
  fi
  if [ -d "$srcRoot/.fingerprint" ]; then
    cp -R "$srcRoot/.fingerprint" "$dstRoot/"
    copied=1
  fi
}
if [ "$needsHostArtifacts" -eq 1 ]; then
  copy_install_layout "$CARGO_TARGET_DIR/$buildMode" "$out"
fi
for targetTriple in "${targetTriples[@]}"; do
  mkdir -p "$out/$targetTriple"
  copy_install_layout "$CARGO_TARGET_DIR/$targetTriple/$buildMode" "$out/$targetTriple"
done
if [ "$copied" -eq 0 ]; then
  touch "$out/.nix-cargo-empty"
fi
"#,
    );

    script
}

#[cfg(test)]
mod tests {
    use super::{
        build_package_phase_plan, command_extern_artifacts, metadata_emit_value, DependencyPhase,
    };
    use crate::model::{CommandEnv, CommandSpec, Plan, PlanPackage, Unit};

    fn command(args: &[&str]) -> CommandSpec {
        CommandSpec {
            cwd: None,
            env: Vec::<CommandEnv>::new(),
            program: "rustc".to_string(),
            args: args.iter().map(|value| (*value).to_string()).collect(),
        }
    }

    fn unit(
        package_key: &str,
        package_name: &str,
        target_kind: &str,
        args: &[&str],
        package_dependencies: &[&str],
    ) -> Unit {
        Unit {
            unit_id: format!("{package_key}:{target_kind}"),
            package_key: package_key.to_string(),
            package_name: package_name.to_string(),
            package_version: "0.1.0".to_string(),
            target_name: package_name.to_string(),
            target_kind: target_kind.to_string(),
            compile_mode: "Build".to_string(),
            target_triple: None,
            build_script_binary: None,
            package_dependencies: package_dependencies
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            command: command(args),
        }
    }

    #[test]
    fn metadata_emit_value_keeps_only_metadata_and_dep_info() {
        assert_eq!(
            metadata_emit_value("dep-info,metadata,link"),
            "dep-info,metadata".to_string()
        );
    }

    #[test]
    fn command_extern_artifacts_parses_split_extern_args() {
        let parsed = command_extern_artifacts(&command(&[
            "--extern",
            "corelib=@@NIXCARGO_TARGET@@/debug/deps/libcorelib-1234abcd.rmeta",
        ]));
        assert_eq!(
            parsed,
            vec![(
                "corelib".to_string(),
                "@@NIXCARGO_TARGET@@/debug/deps/libcorelib-1234abcd.rmeta".to_string()
            )]
        );
    }

    #[test]
    fn package_phase_plan_uses_metadata_for_lib_deps_and_full_for_bin_deps() {
        let dep_key = "dep v0.1.0 (/tmp/dep)";
        let app_key = "app v0.1.0 (/tmp/app)";
        let dep_package = PlanPackage {
            key: dep_key.to_string(),
            name: "dep".to_string(),
            version: "0.1.0".to_string(),
            source: "/tmp/dep".to_string(),
            manifest_path: "/tmp/dep/Cargo.toml".to_string(),
            cargo_home_rel_manifest_path: None,
            lock_checksum: None,
            workspace_member: true,
            dependencies: Vec::new(),
        };
        let app_package = PlanPackage {
            key: app_key.to_string(),
            name: "app".to_string(),
            version: "0.1.0".to_string(),
            source: "/tmp/app".to_string(),
            manifest_path: "/tmp/app/Cargo.toml".to_string(),
            cargo_home_rel_manifest_path: None,
            lock_checksum: None,
            workspace_member: true,
            dependencies: vec![dep_key.to_string()],
        };
        let plan = Plan {
            workspace_root: "/tmp".to_string(),
            manifest_path: "/tmp/Cargo.toml".to_string(),
            cargo_home: "/tmp/ch".to_string(),
            target_dir: "/tmp/target".to_string(),
            target_triple: None,
            packages: vec![dep_package.clone(), app_package.clone()],
            units: Vec::new(),
        };
        let units = vec![
            unit(
                app_key,
                "app",
                "lib",
                &[
                    "--crate-type",
                    "lib",
                    "--emit=dep-info,metadata,link",
                    "--extern",
                    "dep=@@NIXCARGO_TARGET@@/debug/deps/libdep-1234abcd.rmeta",
                ],
                &[dep_key],
            ),
            unit(
                app_key,
                "app",
                "bin",
                &[
                    "--crate-type",
                    "bin",
                    "--emit=dep-info,link",
                    "--extern",
                    "dep=@@NIXCARGO_TARGET@@/debug/deps/libdep-1234abcd.rlib",
                ],
                &[dep_key],
            ),
        ];

        let phase_plan = build_package_phase_plan(&plan, &app_package, &units);

        assert!(phase_plan.has_metadata_phase());
        assert_eq!(phase_plan.metadata_units.len(), 1);
        assert_eq!(
            phase_plan.metadata_units[0].command.args,
            vec![
                "--crate-type".to_string(),
                "lib".to_string(),
                "--emit=dep-info,metadata".to_string(),
                "--extern".to_string(),
                "dep=@@NIXCARGO_TARGET@@/debug/deps/libdep-1234abcd.rmeta".to_string(),
            ]
        );
        assert_eq!(
            phase_plan.metadata_dependencies.get(dep_key),
            Some(&DependencyPhase::Metadata)
        );
        assert_eq!(
            phase_plan.full_dependencies.get(dep_key),
            Some(&DependencyPhase::Full)
        );
    }
}
