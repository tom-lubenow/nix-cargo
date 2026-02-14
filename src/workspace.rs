use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cargo::core::Workspace;
use cargo::GlobalContext;

use crate::model::{PackageSummary, WorkspaceSummary};

pub fn summarize_workspace(manifest_path: Option<&Path>) -> Result<WorkspaceSummary> {
    let gctx = GlobalContext::default().context("failed to initialize cargo global context")?;
    let manifest_path = resolve_manifest_path(manifest_path)?;
    let ws = Workspace::new(&manifest_path, &gctx).with_context(|| {
        format!(
            "failed to load cargo workspace from {}",
            manifest_path.display()
        )
    })?;

    let workspace_root_path = ws.root().to_path_buf();
    let workspace_root = ws.root().display().to_string();
    let root_manifest_path = ws.root_manifest().to_path_buf();
    let manifest_path = root_manifest_path.display().to_string();

    let workspace_member_names: BTreeSet<String> = ws
        .members()
        .map(|pkg| pkg.name().to_string())
        .collect();

    let packages = ws
        .members()
        .map(|package| {
            let manifest_path = package.manifest_path().to_path_buf();
            let manifest_path_display = manifest_path.display().to_string();
            let relative_manifest_path = manifest_path
                .strip_prefix(&workspace_root_path)
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_else(|_| manifest_path_display.clone());

            let dependency_names = package
                .dependencies()
                .iter()
                .map(|dep| dep.package_name().to_string())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();

            let workspace_dependencies = package
                .dependencies()
                .iter()
                .map(|dep| dep.package_name().to_string())
                .filter(|dep_name| workspace_member_names.contains(dep_name))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();

            let targets = package
                .targets()
                .iter()
                .filter(|target| !target.proc_macro())
                .map(|target| target.name().to_string())
                .collect();

            PackageSummary {
                id: package.package_id().to_string(),
                name: package.name().to_string(),
                version: package.version().to_string(),
                manifest_path: manifest_path_display,
                relative_manifest_path,
                targets,
                dependency_names,
                workspace_dependencies,
            }
        })
        .collect();

    Ok(WorkspaceSummary {
        manifest_path,
        workspace_root,
        packages: topologically_sort_packages(packages),
    })
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

fn topologically_sort_packages(packages: Vec<PackageSummary>) -> Vec<PackageSummary> {
    if packages.len() <= 1 {
        return packages;
    }

    let mut name_to_index: HashMap<String, usize> = HashMap::with_capacity(packages.len());
    for (index, package) in packages.iter().enumerate() {
        name_to_index.insert(package.name.clone(), index);
    }

    let mut indegree = vec![0usize; packages.len()];
    let mut dependents: HashMap<usize, Vec<usize>> = HashMap::new();

    for (idx, package) in packages.iter().enumerate() {
        for dependency in &package.workspace_dependencies {
            if let Some(dep_idx) = name_to_index.get(dependency) {
                indegree[idx] += 1;
                dependents.entry(*dep_idx).or_default().push(idx);
            }
        }
    }

    let mut ready: BTreeSet<String> = packages
        .iter()
        .filter_map(|package| (indegree[name_to_index[&package.name]] == 0).then_some(package.name.clone()))
        .collect();

    let mut sorted = Vec::with_capacity(packages.len());
    let mut emitted: HashSet<String> = HashSet::with_capacity(packages.len());

    while let Some(name) = ready.iter().next().cloned() {
        let _ = ready.remove(&name);
        let idx = name_to_index[&name];
        let package = packages[idx].clone();

        if emitted.insert(name.clone()) {
            sorted.push(package);
        }

        for dependent_idx in dependents.get(&idx).into_iter().flatten() {
            let dependent_package = &packages[*dependent_idx];
            indegree[*dependent_idx] -= 1;

            if indegree[*dependent_idx] == 0 {
                ready.insert(dependent_package.name.clone());
            }
        }
    }

    if sorted.len() == packages.len() {
        return sorted;
    }

    let mut remaining: Vec<PackageSummary> = packages
        .into_iter()
        .filter(|package| !emitted.contains(&package.name))
        .collect();
    remaining.sort_by(|a, b| a.name.cmp(&b.name));
    sorted.extend(remaining);
    sorted
}
