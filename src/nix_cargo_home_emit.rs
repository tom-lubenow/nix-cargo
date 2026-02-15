use std::collections::BTreeSet;
use std::fmt::Write;

use crate::cargo_home::CargoHomeMaterializationPlan;
use crate::nix_string::nix_escape;

pub(crate) fn append_cargo_home_section(
    out: &mut String,
    cargo_home_plan: &CargoHomeMaterializationPlan,
) {
    for crate_entry in &cargo_home_plan.registry_crates {
        let _ = writeln!(
            out,
            "  {} = pkgs.fetchurl {{ url = \"{}\"; hash = \"{}\"; }};",
            crate_entry.archive_binding,
            nix_escape(&crate_entry.download_url),
            nix_escape(&crate_entry.hash_sri),
        );
    }

    let mut emitted_git_bindings = BTreeSet::new();
    for git_entry in &cargo_home_plan.git_crates {
        if !emitted_git_bindings.insert(git_entry.source_binding.clone()) {
            continue;
        }
        let _ = writeln!(
            out,
            "  {} = if builtins.hasAttr \"{}\" gitSourceHashes then pkgs.fetchgit {{ url = \"{}\"; rev = \"{}\"; hash = builtins.getAttr \"{}\" gitSourceHashes; }} else if allowImpureGitFetch then builtins.fetchGit {{ url = \"{}\"; rev = \"{}\"; }} else throw \"nix-cargo: missing git hash for source {}. Pass gitSourceHashes or set allowImpureGitFetch = true.\";",
            git_entry.source_binding,
            nix_escape(&git_entry.source_key),
            nix_escape(&git_entry.url),
            nix_escape(&git_entry.rev),
            nix_escape(&git_entry.source_key),
            nix_escape(&git_entry.url),
            nix_escape(&git_entry.rev),
            nix_escape(&git_entry.source_key),
        );
    }

    out.push_str("  materializedCargoHome = pkgs.runCommand \"nix-cargo-home\" {\n");
    out.push_str("    nativeBuildInputs = [ pkgs.coreutils pkgs.gnutar pkgs.gzip ];\n");
    out.push_str("  } ''\n");
    out.push_str("    set -euo pipefail\n");
    out.push_str("    mkdir -p \"$out\" \"$out/registry/src\" \"$out/git/checkouts\" \"$out/git/db\"\n");
    for crate_entry in &cargo_home_plan.registry_crates {
        let _ = writeln!(
            out,
            "    mkdir -p \"$out/{}\"",
            nix_escape(&crate_entry.registry_src_parent)
        );
        let _ = writeln!(
            out,
            "    tar -xzf ${{{}}} -C \"$out/{}\"",
            crate_entry.archive_binding,
            nix_escape(&crate_entry.registry_src_parent)
        );
    }
    for git_entry in &cargo_home_plan.git_crates {
        let _ = writeln!(
            out,
            "    mkdir -p \"$out/{}\"",
            nix_escape(&git_entry.destination_parent_rel),
        );
        if let Some(repo_subpath) = git_entry.repo_subpath.as_deref() {
            let _ = writeln!(
                out,
                "    cp -R \"${{{}}}/{}/.\" \"$out/{}/\"",
                git_entry.source_binding,
                nix_escape(repo_subpath),
                nix_escape(&git_entry.destination_parent_rel),
            );
        } else {
            let _ = writeln!(
                out,
                "    cp -R \"${{{}}}/.\" \"$out/{}/\"",
                git_entry.source_binding,
                nix_escape(&git_entry.destination_parent_rel),
            );
        }
    }
    out.push_str("  '';\n");

    let unsupported_packages = cargo_home_plan
        .unsupported_package_keys
        .iter()
        .map(|value| format!("\"{}\"", nix_escape(value)))
        .collect::<Vec<_>>()
        .join(" ");
    let _ = writeln!(
        out,
        "  unsupportedCargoHomePackages = [ {} ];",
        unsupported_packages
    );
    out.push_str("  effectiveCargoHome =\n");
    out.push_str("    if cargoHome != null then cargoHome\n");
    out.push_str("    else if unsupportedCargoHomePackages == [ ] then materializedCargoHome\n");
    out.push_str("    else throw \"nix-cargo: cannot auto-materialize cargoHome for packages: ${builtins.concatStringsSep \", \" unsupportedCargoHomePackages}. Pass cargoHome override.\";\n");
}

