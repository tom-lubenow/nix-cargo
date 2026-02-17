use anyhow::{anyhow, Context, Result};
use nix_libstore::derivation::Derivation;
use nix_libstore::derived_path::SingleDerivedPath;
use nix_libstore::store_path::StorePath;
use std::ffi::OsStr;
use std::io::Write;
use std::process::{Command, Output};
use std::str;

/// Configuration for Nix store operations
#[derive(Debug, Clone)]
pub struct StoreConfig {
    /// Path to the Nix executable
    pub nix_tool: String,

    /// Extra arguments to pass to Nix commands
    pub extra_args: Vec<String>,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            nix_tool: "nix".to_string(),
            extra_args: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct NixTool {
    config: StoreConfig,
}

impl NixTool {
    pub fn new(config: StoreConfig) -> Self {
        NixTool { config }
    }

    pub fn build(&self, derived_paths: &[SingleDerivedPath]) -> Result<Vec<StorePath>> {
        let installables: Vec<String> = derived_paths.iter().map(|p| p.to_string()).collect();
        let output = Command::new(&self.config.nix_tool)
            .args(&self.config.extra_args)
            .args(["build", "-L", "--no-link", "--print-out-paths"])
            .args(&installables)
            .stderr(std::process::Stdio::inherit())
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("Failed to build:\n{}", stderr));
        }

        let stdout = str::from_utf8(&output.stdout)?;
        let store_paths: Vec<StorePath> = stdout
            .lines()
            .map(|line| StorePath::new(line.trim()))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(store_paths)
    }

    /// Add a file to the Nix store
    pub fn store_add(&self, path: &std::path::Path) -> Result<StorePath> {
        self.store_add_named(path, None)
    }

    /// Add a file or directory to the Nix store with an optional store name.
    pub fn store_add_named(
        &self,
        path: &std::path::Path,
        name: Option<&str>,
    ) -> Result<StorePath> {
        let mut command = Command::new(&self.config.nix_tool);
        command.args(&self.config.extra_args).args(["store", "add"]);
        if let Some(name) = name {
            command.args(["--name", name]);
        }
        command.arg(path);
        let output = command.output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!(
                "Failed to store add {}: {}",
                path.to_string_lossy(),
                stderr
            ));
        }

        let store_path_str = String::from_utf8(output.stdout)
            .context("Failed to parse command output")?
            .trim()
            .to_string();

        StorePath::new(store_path_str).context("Failed to parse store path")
    }

    pub fn derivation_show(&self, drv_path: &StorePath) -> Result<Output> {
        self.run_nix_command(&["derivation", "show", &drv_path.to_string()])
            .map_err(|err| {
                anyhow!(
                    "Failed to derivation show {}: {}",
                    &drv_path.to_string(),
                    err
                )
            })
    }

    /// Add a derivation to the Nix store
    pub fn derivation_add(&self, drv: &Derivation) -> Result<StorePath> {
        let mut candidate = drv.clone();

        for _attempt in 0..4 {
            let output = self.run_derivation_add_once(&candidate)?;

            if output.status.success() {
                let store_path_str = String::from_utf8(output.stdout)
                    .context("Failed to parse command output")?
                    .trim()
                    .to_string();
                return StorePath::new(store_path_str).context("Failed to parse store path");
            }

            let stderr = String::from_utf8_lossy(&output.stderr);
            if let Some(expected_store_path) = extract_expected_output_store_path(&stderr) {
                if candidate.outputs.is_empty() {
                    break;
                }

                let mut output_names = candidate.outputs.keys().cloned().collect::<Vec<_>>();
                output_names.sort();
                if output_names.iter().any(|name| name == "out") {
                    candidate.set_output_path("out", &expected_store_path);
                    candidate.set_env("out", &expected_store_path);
                } else if let Some(first) = output_names.first() {
                    candidate.set_output_path(first, &expected_store_path);
                    candidate.set_env(first, &expected_store_path);
                }
                continue;
            }

            return Err(anyhow!("Failed to derivation add {}: {}", candidate.name, stderr));
        }

        Err(anyhow!(
            "Failed to derivation add {}: unable to resolve output path mismatch",
            candidate.name
        ))
    }

    fn run_derivation_add_once(&self, drv: &Derivation) -> Result<Output> {
        let json = drv.to_json()?;

        let mut command = Command::new(&self.config.nix_tool);
        command
            .args(&self.config.extra_args)
            .args(["derivation", "add"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = command.spawn()?;
        child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Failed to open stdin"))?
            .write_all(json.as_bytes())?;

        child.wait_with_output().context("Failed to run derivation add")
    }

    /// Run a Nix command and return its output
    fn run_nix_command<S: AsRef<OsStr>>(&self, args: &[S]) -> Result<Output> {
        let output = Command::new(&self.config.nix_tool)
            .args(&self.config.extra_args)
            .args(args)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("Nix command failed:\n{}", stderr));
        }

        Ok(output)
    }
}

fn extract_expected_output_store_path(stderr: &str) -> Option<String> {
    let marker = "should be '/nix/store/";
    let idx = stderr.find(marker)?;
    let tail = &stderr[(idx + "should be '".len())..];
    let end = tail.find('\'')?;
    Some(tail[..end].to_string())
}
