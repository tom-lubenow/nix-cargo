use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use cargo::core::compiler::{CompileMode, DefaultExecutor, Executor, UserIntent};
use cargo::core::manifest::Target;
use cargo::core::{PackageId, Verbosity};
use cargo::ops::{self, CompileOptions};
use cargo::util::interning::InternedString;
use cargo::{CargoResult, GlobalContext};
use cargo_util::ProcessBuilder;
use serde::Deserialize;

use crate::model::{
    CommandEnv, CommandSpec, Plan, PlanPackage, Unit, PATH_MARKER_CARGO_BIN,
    PATH_MARKER_CARGO_HOME, PATH_MARKER_RUSTC, PATH_MARKER_SRC, PATH_MARKER_TARGET,
};

pub fn build_plan(
    manifest_path: Option<&Path>,
    release: bool,
    target_triple: Option<&str>,
) -> Result<Plan> {
    let gctx = GlobalContext::default().context("failed to initialize cargo global context")?;
    gctx.shell().set_verbosity(Verbosity::Quiet);

    let manifest_path = resolve_manifest_path(manifest_path)?;
    let ws = cargo::core::Workspace::new(&manifest_path, &gctx).with_context(|| {
        format!(
            "failed to load cargo workspace from {}",
            manifest_path.display()
        )
    })?;

    let workspace_root = ws.root().display().to_string();
    let target_dir = ws.target_dir().as_path_unlocked().display().to_string();
    let cargo_home_path = gctx.home().as_path_unlocked().to_path_buf();
    let cargo_home = cargo_home_path.display().to_string();
    let rustc_path = gctx
        .load_global_rustc(Some(&ws))
        .context("failed to load global rustc")?
        .path
        .display()
        .to_string();
    let rewrite_context = RewriteContext {
        workspace_root: workspace_root.clone(),
        target_dir: target_dir.clone(),
        cargo_home: cargo_home.clone(),
        rustc_path,
    };
    let lockfile_checksums = load_lockfile_checksums(ws.root())?;

    let (package_set, resolve) =
        ops::resolve_ws(&ws, false).context("failed to resolve workspace dependency graph")?;
    let package_order = resolve.sort();
    let _ = package_set
        .get_many(package_order.iter().cloned())
        .context("failed to materialize resolved cargo packages")?;

    let workspace_members: BTreeSet<String> = ws
        .members()
        .map(|pkg| pkg.package_id().to_string())
        .collect();

    let mut package_dependencies: HashMap<String, Vec<String>> = HashMap::new();
    let mut packages = Vec::with_capacity(package_order.len());
    for package_id in package_order.iter().cloned() {
        let key = package_id.to_string();
        let dependencies = resolve
            .deps(package_id)
            .map(|(dep_id, _)| dep_id.to_string())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();

        package_dependencies.insert(key.clone(), dependencies.clone());

        let package = package_set
            .get_one(package_id)
            .with_context(|| format!("failed to resolve package metadata for {key}"))?;
        let source = package_id.source_id().to_string();
        let name = package.name().to_string();
        let version = package.version().to_string();
        let manifest_path = package.manifest_path().display().to_string();
        let cargo_home_rel_manifest_path = package
            .manifest_path()
            .strip_prefix(&cargo_home_path)
            .ok()
            .map(|path| path.to_string_lossy().to_string());
        let lock_checksum = lockfile_checksums
            .get(&LockPackageKey {
                name: name.clone(),
                version: version.clone(),
                source: lock_source_for_lookup(&source),
            })
            .cloned();

        packages.push(PlanPackage {
            key: key.clone(),
            name,
            version,
            source,
            manifest_path,
            cargo_home_rel_manifest_path,
            lock_checksum,
            workspace_member: workspace_members.contains(&key),
            dependencies,
        });
    }

    let mut options = CompileOptions::new(&gctx, UserIntent::Build)
        .context("failed to build cargo compile options")?;
    options.spec = ops::Packages::Default;
    if release {
        options.build_config.requested_profile = InternedString::new("release");
    }
    if let Some(target_triple) = target_triple {
        options.build_config.requested_kinds =
            cargo::core::compiler::CompileKind::from_requested_targets(
                &gctx,
                &[target_triple.to_string()],
            )
            .with_context(|| format!("failed to resolve target triple `{target_triple}`"))?;
    }
    options.build_config.keep_going = true;
    options.build_config.dry_run = false;
    options.build_config.force_rebuild = true;

    let executor = Arc::new(RecordingExecutor::new(true));
    let exec: Arc<dyn Executor> = executor.clone();

    let compile_result = ops::compile_with_exec(&ws, &options, &exec);
    let captured_units = executor.captured_units();
    if let Err(error) = compile_result {
        if captured_units.is_empty() {
            return Err(error).context("cargo compile planning failed");
        }
    }

    let package_versions: HashMap<&str, &str> = packages
        .iter()
        .map(|package| (package.key.as_str(), package.version.as_str()))
        .collect();

    let units = captured_units
        .into_iter()
        .map(|captured| Unit {
            unit_id: captured.unit_id,
            package_key: captured.package_key.clone(),
            package_name: captured.package_name,
            package_version: package_versions
                .get(captured.package_key.as_str())
                .copied()
                .unwrap_or("unknown")
                .to_string(),
            target_name: captured.target_name,
            target_kind: captured.target_kind,
            compile_mode: captured.compile_mode,
            package_dependencies: package_dependencies
                .get(captured.package_key.as_str())
                .cloned()
                .unwrap_or_default(),
            command: normalize_command(captured.command, &rewrite_context),
        })
        .collect();

    Ok(Plan {
        workspace_root: workspace_root.clone(),
        manifest_path: manifest_path.display().to_string(),
        cargo_home,
        target_dir,
        target_triple: target_triple.map(ToOwned::to_owned),
        packages,
        units,
    })
}

#[derive(Debug, Clone)]
struct CapturedUnit {
    unit_id: String,
    package_key: String,
    package_name: String,
    target_name: String,
    target_kind: String,
    compile_mode: String,
    command: CommandSpec,
}

struct RecordingExecutor {
    captured: Mutex<Vec<CapturedUnit>>,
    delegate: DefaultExecutor,
    execute_commands: bool,
}

impl RecordingExecutor {
    fn new(execute_commands: bool) -> Self {
        Self {
            captured: Mutex::new(Vec::new()),
            delegate: DefaultExecutor,
            execute_commands,
        }
    }

    fn captured_units(&self) -> Vec<CapturedUnit> {
        self.captured
            .lock()
            .expect("recording executor mutex poisoned")
            .clone()
    }
}

impl Executor for RecordingExecutor {
    fn force_rebuild(&self, _unit: &cargo::core::compiler::Unit) -> bool {
        true
    }

    fn exec(
        &self,
        cmd: &ProcessBuilder,
        id: PackageId,
        target: &Target,
        mode: CompileMode,
        on_stdout_line: &mut dyn FnMut(&str) -> CargoResult<()>,
        on_stderr_line: &mut dyn FnMut(&str) -> CargoResult<()>,
    ) -> CargoResult<()> {
        let mut captured = self
            .captured
            .lock()
            .expect("recording executor mutex poisoned");
        captured.push(CapturedUnit {
            unit_id: format!("{}:{}:{mode:?}", id, target.name()),
            package_key: id.to_string(),
            package_name: id.name().to_string(),
            target_name: target.name().to_string(),
            target_kind: target.kind().description().to_string(),
            compile_mode: format!("{mode:?}"),
            command: capture_command(cmd),
        });
        drop(captured);

        if self.execute_commands {
            self.delegate
                .exec(cmd, id, target, mode, on_stdout_line, on_stderr_line)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone)]
struct RewriteContext {
    workspace_root: String,
    target_dir: String,
    cargo_home: String,
    rustc_path: String,
}

fn resolve_manifest_path(manifest_path: Option<&Path>) -> Result<PathBuf> {
    let candidate = match manifest_path {
        Some(path) => path.to_path_buf(),
        None => std::env::current_dir()
            .context("failed to read current directory")?
            .join("Cargo.toml"),
    };

    if candidate.is_absolute() {
        return Ok(candidate);
    }

    Ok(std::env::current_dir()
        .context("failed to read current directory")?
        .join(candidate))
}

fn capture_command(cmd: &ProcessBuilder) -> CommandSpec {
    let cwd = cmd.get_cwd().map(|path| path.display().to_string());
    let env = cmd
        .get_envs()
        .iter()
        .filter_map(|(key, value)| {
            value.as_ref().map(|value| CommandEnv {
                key: key.clone(),
                value: value.to_string_lossy().to_string(),
            })
        })
        .collect();

    let program = cmd.get_program().to_string_lossy().to_string();
    let args = cmd
        .get_args()
        .map(|arg| arg.to_string_lossy().to_string())
        .collect();

    CommandSpec {
        cwd,
        env,
        program,
        args,
    }
}

fn normalize_command(mut command: CommandSpec, context: &RewriteContext) -> CommandSpec {
    command.cwd = command
        .cwd
        .map(|cwd| normalize_value(&cwd, context));

    command.env = command
        .env
        .into_iter()
        .map(|entry| normalize_env(entry, context))
        .collect();

    command.program = normalize_program(&command.program, context);
    command.args = command
        .args
        .into_iter()
        .map(|arg| normalize_value(&arg, context))
        .collect();

    command
}

fn normalize_env(mut entry: CommandEnv, context: &RewriteContext) -> CommandEnv {
    if entry.key == "CARGO" {
        entry.value = PATH_MARKER_CARGO_BIN.to_string();
        return entry;
    }
    if entry.key == "RUSTC" {
        entry.value = PATH_MARKER_RUSTC.to_string();
        return entry;
    }

    entry.value = normalize_value(&entry.value, context);
    entry
}

fn normalize_program(program: &str, context: &RewriteContext) -> String {
    if looks_like_rustc(program) || program == context.rustc_path {
        return PATH_MARKER_RUSTC.to_string();
    }

    normalize_value(program, context)
}

fn looks_like_rustc(value: &str) -> bool {
    let candidate = std::path::Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(value);
    candidate == "rustc" || candidate == "rustc.exe"
}

fn normalize_value(value: &str, context: &RewriteContext) -> String {
    let mut rewritten = value.to_string();
    rewritten = replace_prefix(rewritten, &context.target_dir, PATH_MARKER_TARGET);
    rewritten = replace_prefix(rewritten, &context.workspace_root, PATH_MARKER_SRC);
    rewritten = replace_prefix(rewritten, &context.cargo_home, PATH_MARKER_CARGO_HOME);
    if path_like(&context.rustc_path) {
        rewritten = replace_prefix(rewritten, &context.rustc_path, PATH_MARKER_RUSTC);
    }
    rewritten
}

fn replace_prefix(value: String, from: &str, marker: &str) -> String {
    if from.is_empty() {
        return value;
    }
    value.replace(from, marker)
}

fn path_like(value: &str) -> bool {
    value.contains('/') || value.contains('\\')
}

#[derive(Debug, Deserialize)]
struct CargoLockFile {
    package: Option<Vec<CargoLockPackage>>,
}

#[derive(Debug, Deserialize)]
struct CargoLockPackage {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct LockPackageKey {
    name: String,
    version: String,
    source: Option<String>,
}

fn load_lockfile_checksums(workspace_root: &Path) -> Result<HashMap<LockPackageKey, String>> {
    let lock_path = workspace_root.join("Cargo.lock");
    if !lock_path.exists() {
        return Ok(HashMap::new());
    }

    let lock_contents = std::fs::read_to_string(&lock_path)
        .with_context(|| format!("failed to read {}", lock_path.display()))?;
    let lock: CargoLockFile = toml::from_str(&lock_contents)
        .with_context(|| format!("failed to parse {}", lock_path.display()))?;

    let mut checksums = HashMap::new();
    for package in lock.package.unwrap_or_default() {
        let Some(source) = package.source else {
            continue;
        };
        let Some(checksum) = package.checksum else {
            continue;
        };

        checksums.insert(
            LockPackageKey {
                name: package.name,
                version: package.version,
                source: Some(source),
            },
            checksum,
        );
    }

    Ok(checksums)
}

fn lock_source_for_lookup(source: &str) -> Option<String> {
    if source.starts_with("registry+") || source.starts_with("git+") {
        return Some(source.to_string());
    }
    None
}
