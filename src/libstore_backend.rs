use std::collections::{BTreeMap, HashMap};
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
    Plan, PlanPackage, PATH_MARKER_CARGO_BIN, PATH_MARKER_CARGO_HOME, PATH_MARKER_RUSTC,
    PATH_MARKER_SRC, PATH_MARKER_TARGET,
};
use crate::nix_string::{shell_array_literal, shell_single_quote};
use crate::plan_package::{topologically_sorted_packages, units_by_package};
use crate::source_scope::workspace_source_prefixes_by_package;

#[derive(Debug, Clone, Serialize)]
pub struct PackageLayoutInfo {
    pub target_triples: Vec<String>,
    pub needs_host_artifacts: bool,
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
    pub package_layouts: BTreeMap<String, PackageLayoutInfo>,
    #[serde(skip_serializing)]
    package_refs: HashMap<String, SingleDerivedPath>,
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
    let mut package_layouts = BTreeMap::new();
    let mut package_refs: HashMap<String, SingleDerivedPath> = HashMap::new();
    let mut package_output_paths: HashMap<String, String> = HashMap::new();

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

        let dependency_refs = package
            .dependencies
            .iter()
            .filter_map(|key| package_refs.get(key).cloned())
            .collect::<Vec<_>>();
        let dependency_output_paths = package
            .dependencies
            .iter()
            .filter_map(|key| package_output_paths.get(key).cloned())
            .collect::<Vec<_>>();

        let command_script = render_command_script(&units);
        let build_script = render_package_builder_script(
            plan,
            package,
            &layout,
            &toolchain,
            &source_store_path,
            cargo_home_snapshot.as_ref(),
            &dependency_output_paths,
            &command_script,
            release_mode,
        );

        let drv_name = format!(
            "nix-cargo-{}-{}",
            sanitize_derivation_component(&package.name),
            package_index
        );
        let mut derivation = Derivation::new(&drv_name, &system, &toolchain.bash_builder());
        derivation
            .add_arg("-euo")
            .add_arg("pipefail")
            .add_arg("-c")
            .add_arg(&build_script)
            .set_env("PATH", &toolchain.path_env())
            .add_output("out", None, None, None);
        derivation.add_input_src(&source_store_path);
        if let Some(cargo_home_store) = cargo_home_snapshot.as_ref() {
            derivation.add_input_src(cargo_home_store);
        }
        for dep in &dependency_refs {
            derivation.add_derived_path(dep);
        }
        toolchain.add_inputs(&mut derivation);

        let drv_path = nix_tool
            .derivation_add(&derivation)
            .with_context(|| format!("failed to add derivation for {}", package.key))?;
        let package_ref =
            SingleDerivedPath::Built(SingleDerivedPathBuilt::new(drv_path.clone(), "out".to_string()));
        let output_path = derivation_output_path(&nix_tool, &drv_path, "out")
            .with_context(|| format!("failed to resolve output path for {}", package.key))?;

        package_names.insert(package.key.clone(), package.name.clone());
        package_derivations.insert(package.key.clone(), drv_path.to_string());
        package_installables.insert(package.key.clone(), package_ref.to_string());
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
        package_layouts,
        package_refs,
    })
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
            bash: resolve_nixpkgs_tool("bash")?,
            coreutils: resolve_nixpkgs_tool("coreutils")?,
            cargo: resolve_nixpkgs_tool("cargo")?,
            rustc: resolve_nixpkgs_tool("rustc")?,
            pkg_config: resolve_nixpkgs_tool("pkg-config")?,
            cc: resolve_nixpkgs_tool("stdenv.cc")?,
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

fn resolve_nixpkgs_tool(attr: &str) -> Result<NixpkgsTool> {
    let drv_expr = format!("let pkgs = import <nixpkgs> {{}}; in pkgs.{attr}.drvPath");
    let out_expr = format!("let pkgs = import <nixpkgs> {{}}; in pkgs.{attr}.outPath");
    let drv_path = StorePath::new(nix_eval_raw(&drv_expr)?)
        .with_context(|| format!("invalid drvPath for nixpkgs attr `{attr}`"))?;
    let out_path = StorePath::new(nix_eval_raw(&out_expr)?)
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
