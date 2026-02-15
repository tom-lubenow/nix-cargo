use std::collections::HashMap;
use std::fmt::Write;

use crate::command_layout::PackageLayoutRequirements;
use crate::command_script::render_command_script;
use crate::model::{PlanPackage, Unit};
use crate::nix_string::{
    nix_bool, nix_escape, nix_optional_string, nix_string_list,
};

pub(crate) fn append_crate_plan_section(
    out: &mut String,
    ordered_packages: &[PlanPackage],
    units_by_package: &HashMap<String, Vec<Unit>>,
    package_layout: &HashMap<String, PackageLayoutRequirements>,
    source_prefixes_by_package: &HashMap<String, Vec<String>>,
) {
    out.push_str("  cratePlan = [\n");
    for package in ordered_packages {
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
        let package_source_prefixes = source_prefixes_by_package
            .get(package.key.as_str())
            .cloned()
            .unwrap_or_default();
        let command_script = render_command_script(package_units);
        let _ = writeln!(
            out,
            "    {{ key = \"{}\"; name = \"{}\"; version = \"{}\"; source = \"{}\"; lockChecksum = {}; cargoHomeRelManifestPath = {}; workspaceMember = {}; dependencies = {}; workspaceSourcePrefixes = {}; targetTriples = {}; needsHostArtifacts = {}; commandScript = \"{}\"; }}",
            nix_escape(&package.key),
            nix_escape(&package.name),
            nix_escape(&package.version),
            nix_escape(&package.source),
            nix_optional_string(package.lock_checksum.as_deref()),
            nix_optional_string(package.cargo_home_rel_manifest_path.as_deref()),
            nix_bool(package.workspace_member),
            nix_string_list(&package.dependencies),
            nix_string_list(&package_source_prefixes),
            nix_string_list(&target_triples),
            nix_bool(needs_host_artifacts),
            nix_escape(&command_script),
        );
    }
    out.push_str("  ];\n");
}
