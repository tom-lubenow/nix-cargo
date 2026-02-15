use serde::Serialize;

pub const PATH_MARKER_SRC: &str = "@@NIXCARGO_SRC@@";
pub const PATH_MARKER_TARGET: &str = "@@NIXCARGO_TARGET@@";
pub const PATH_MARKER_CARGO_HOME: &str = "@@NIXCARGO_CARGO_HOME@@";
pub const PATH_MARKER_RUSTC: &str = "@@NIXCARGO_RUSTC@@";
pub const PATH_MARKER_CARGO_BIN: &str = "@@NIXCARGO_CARGO@@";

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceSummary {
    pub manifest_path: String,
    pub workspace_root: String,
    pub packages: Vec<PackageSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub manifest_path: String,
    pub relative_manifest_path: String,
    pub targets: Vec<String>,
    pub dependency_names: Vec<String>,
    pub workspace_dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub workspace_root: String,
    pub manifest_path: String,
    pub cargo_home: String,
    pub target_dir: String,
    pub target_triple: Option<String>,
    pub packages: Vec<PlanPackage>,
    pub units: Vec<Unit>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Unit {
    pub unit_id: String,
    pub package_key: String,
    pub package_name: String,
    pub package_version: String,
    pub target_name: String,
    pub target_kind: String,
    pub compile_mode: String,
    pub target_triple: Option<String>,
    pub package_dependencies: Vec<String>,
    pub command: CommandSpec,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanPackage {
    pub key: String,
    pub name: String,
    pub version: String,
    pub source: String,
    pub manifest_path: String,
    pub cargo_home_rel_manifest_path: Option<String>,
    pub lock_checksum: Option<String>,
    pub workspace_member: bool,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandSpec {
    pub cwd: Option<String>,
    pub env: Vec<CommandEnv>,
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandEnv {
    pub key: String,
    pub value: String,
}
