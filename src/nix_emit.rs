use crate::cargo_home::build_cargo_home_materialization_plan;
use crate::command_layout::package_layout_by_key;
use crate::model::Plan;
use crate::nix_string::{
    nix_escape,
};
use crate::nix_cargo_home_emit::append_cargo_home_section;
use crate::nix_crate_plan_emit::append_crate_plan_section;
use crate::nix_emit_model::build_rendered_package_plans;
use crate::nix_header_emit::append_preamble;
use crate::nix_package_derivation_emit::append_package_derivations_section;
use crate::nix_public_attrs_emit::append_public_attrs_section;
use crate::plan_package::{topologically_sorted_packages, units_by_package};
use crate::source_scope::workspace_source_prefixes_by_package;

pub fn render_nix_expression(plan: &Plan, release_mode: bool) -> String {
    let mut out = String::new();
    let ordered_packages = topologically_sorted_packages(plan);
    let units_by_package = units_by_package(plan);
    let package_layout = package_layout_by_key(plan);
    let source_prefixes_by_package = workspace_source_prefixes_by_package(plan);
    let cargo_home_plan = build_cargo_home_materialization_plan(plan);
    let rendered_packages = build_rendered_package_plans(
        &ordered_packages,
        &units_by_package,
        &package_layout,
        &source_prefixes_by_package,
    );
    let release_default = if release_mode { "true" } else { "false" };
    let default_src = if plan.workspace_root.is_empty() {
        String::from("builtins.path { path = ./.; name = \"nix-cargo-src\"; }")
    } else {
        format!(
            "builtins.path {{ path = \"{}\"; name = \"nix-cargo-src\"; }}",
            nix_escape(&plan.workspace_root)
        )
    };

    append_preamble(&mut out, plan, &default_src, release_default);

    append_cargo_home_section(&mut out, &cargo_home_plan);
    out.push_str("  emptySrc = pkgs.runCommand \"nix-cargo-empty-src\" {} ''\n");
    out.push_str("    mkdir -p \"$out\"\n");
    out.push_str("  '';\n");

    append_crate_plan_section(&mut out, &rendered_packages);

    append_package_derivations_section(&mut out);
    append_public_attrs_section(&mut out);

    out
}
