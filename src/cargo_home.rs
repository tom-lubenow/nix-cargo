use std::collections::HashMap;
use std::path::Path;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;

use crate::model::{Plan, PlanPackage};

/// Plan for auto-materializing Cargo home sources when `cargoHome = null`.
#[derive(Debug, Clone)]
pub(crate) struct CargoHomeMaterializationPlan {
    pub(crate) registry_crates: Vec<RegistryCrateMaterialization>,
    pub(crate) git_crates: Vec<GitCrateMaterialization>,
    pub(crate) unsupported_package_keys: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct RegistryCrateMaterialization {
    pub(crate) archive_binding: String,
    pub(crate) registry_src_parent: String,
    pub(crate) download_url: String,
    pub(crate) hash_sri: String,
}

#[derive(Debug, Clone)]
pub(crate) struct GitCrateMaterialization {
    pub(crate) source_binding: String,
    pub(crate) source_key: String,
    pub(crate) url: String,
    pub(crate) rev: String,
    pub(crate) destination_parent_rel: String,
    pub(crate) repo_subpath: Option<String>,
}

/// Build a deterministic Cargo home materialization plan from resolved packages.
pub(crate) fn build_cargo_home_materialization_plan(plan: &Plan) -> CargoHomeMaterializationPlan {
    let mut registry_crates = Vec::new();
    let mut git_crates = Vec::new();
    let mut git_source_bindings: HashMap<String, String> = HashMap::new();
    let mut unsupported_package_keys = Vec::new();

    for package in &plan.packages {
        let Some(rel_manifest_path) = package.cargo_home_rel_manifest_path.as_deref() else {
            continue;
        };

        if package.source.starts_with("registry+") {
            if !is_crates_io_registry_source(&package.source) {
                unsupported_package_keys.push(package.key.clone());
                continue;
            }

            let Some(checksum) = package.lock_checksum.as_deref() else {
                unsupported_package_keys.push(package.key.clone());
                continue;
            };
            let Some(hash_sri) = sha256_hex_to_sri(checksum) else {
                unsupported_package_keys.push(package.key.clone());
                continue;
            };

            let manifest = Path::new(rel_manifest_path);
            let Some(crate_dir) = manifest.parent() else {
                unsupported_package_keys.push(package.key.clone());
                continue;
            };
            let Some(registry_src_parent) = crate_dir.parent() else {
                unsupported_package_keys.push(package.key.clone());
                continue;
            };

            let binding = format!("cargoRegistryArchive{}", registry_crates.len());
            registry_crates.push(RegistryCrateMaterialization {
                archive_binding: binding,
                registry_src_parent: registry_src_parent.to_string_lossy().to_string(),
                download_url: crates_io_archive_url(package),
                hash_sri,
            });
            continue;
        }

        if package.source.starts_with("git+") {
            let Some(source) = parse_git_source(&package.source) else {
                unsupported_package_keys.push(package.key.clone());
                continue;
            };
            let Some(layout) = parse_git_checkout_layout(rel_manifest_path) else {
                unsupported_package_keys.push(package.key.clone());
                continue;
            };

            let binding = match git_source_bindings.get(&package.source) {
                Some(binding) => binding.clone(),
                None => {
                    let binding = format!("cargoGitSource{}", git_source_bindings.len());
                    git_source_bindings.insert(package.source.clone(), binding.clone());
                    binding
                }
            };
            git_crates.push(GitCrateMaterialization {
                source_binding: binding,
                source_key: package.source.clone(),
                url: source.url,
                rev: source.rev,
                destination_parent_rel: layout.destination_parent_rel,
                repo_subpath: layout.repo_subpath,
            });
            continue;
        }
    }

    CargoHomeMaterializationPlan {
        registry_crates,
        git_crates,
        unsupported_package_keys,
    }
}

#[derive(Debug, Clone)]
struct ParsedGitSource {
    url: String,
    rev: String,
}

fn parse_git_source(source: &str) -> Option<ParsedGitSource> {
    let raw = source.strip_prefix("git+")?;
    let (before_fragment, fragment) = match raw.split_once('#') {
        Some((base, hash)) => (base, Some(hash)),
        None => (raw, None),
    };
    let (url, query) = match before_fragment.split_once('?') {
        Some((url, query)) => (url, Some(query)),
        None => (before_fragment, None),
    };

    let mut rev = None;
    if let Some(query) = query {
        for part in query.split('&') {
            if let Some(value) = part.strip_prefix("rev=") {
                rev = Some(value.to_string());
                break;
            }
        }
    }

    if rev.is_none() {
        if let Some(fragment) = fragment {
            if !fragment.is_empty() {
                rev = Some(fragment.to_string());
            }
        }
    }

    Some(ParsedGitSource {
        url: url.to_string(),
        rev: rev?,
    })
}

#[derive(Debug, Clone)]
struct GitCheckoutLayout {
    destination_parent_rel: String,
    repo_subpath: Option<String>,
}

fn parse_git_checkout_layout(rel_manifest_path: &str) -> Option<GitCheckoutLayout> {
    let path = Path::new(rel_manifest_path);
    let components = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();

    if components.len() < 5 {
        return None;
    }
    if components.first().map(|s| s.as_str()) != Some("git") {
        return None;
    }
    if components.get(1).map(|s| s.as_str()) != Some("checkouts") {
        return None;
    }
    if components.last().map(|s| s.as_str()) != Some("Cargo.toml") {
        return None;
    }

    let destination_parent_rel = path
        .parent()
        .map(|parent| parent.to_string_lossy().to_string())?;

    let repo_subpath = if components.len() > 5 {
        Some(components[4..components.len() - 1].join("/"))
    } else {
        None
    };

    Some(GitCheckoutLayout {
        destination_parent_rel,
        repo_subpath,
    })
}

fn is_crates_io_registry_source(source: &str) -> bool {
    source.contains("crates.io-index") || source.contains("index.crates.io")
}

fn crates_io_archive_url(package: &PlanPackage) -> String {
    format!(
        "https://static.crates.io/crates/{name}/{name}-{version}.crate",
        name = package.name,
        version = package.version
    )
}

fn sha256_hex_to_sri(hex: &str) -> Option<String> {
    if hex.len() != 64 {
        return None;
    }

    let mut bytes = Vec::with_capacity(32);
    let hex = hex.as_bytes();
    let mut index = 0;
    while index < hex.len() {
        let hi = hex_nibble(hex[index])?;
        let lo = hex_nibble(hex[index + 1])?;
        bytes.push((hi << 4) | lo);
        index += 2;
    }

    Some(format!("sha256-{}", BASE64_STANDARD.encode(bytes)))
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
