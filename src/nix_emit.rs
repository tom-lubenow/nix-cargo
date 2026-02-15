use std::collections::BTreeSet;
use std::fmt::Write;

use crate::cargo_home::{build_cargo_home_materialization_plan, CargoHomeMaterializationPlan};
use crate::command_script::render_command_script;
use crate::command_layout::package_layout_by_key;
use crate::model::{
    Plan, PATH_MARKER_CARGO_BIN, PATH_MARKER_CARGO_HOME, PATH_MARKER_RUSTC, PATH_MARKER_SRC,
    PATH_MARKER_TARGET,
};
use crate::nix_string::{
    nix_bool, nix_escape, nix_optional_string, nix_string_list,
};
use crate::plan_package::{commands_by_package, topologically_sorted_packages};
use crate::source_scope::workspace_source_prefixes_by_package;

pub fn render_nix_expression(plan: &Plan, release_mode: bool) -> String {
    let mut out = String::new();
    let ordered_packages = topologically_sorted_packages(plan);
    let commands_by_package = commands_by_package(plan);
    let package_layout = package_layout_by_key(plan);
    let source_prefixes_by_package = workspace_source_prefixes_by_package(plan);
    let cargo_home_plan = build_cargo_home_materialization_plan(plan);
    let release_default = if release_mode { "true" } else { "false" };
    let default_src = if plan.workspace_root.is_empty() {
        String::from("builtins.path { path = ./.; name = \"nix-cargo-src\"; }")
    } else {
        format!(
            "builtins.path {{ path = \"{}\"; name = \"nix-cargo-src\"; }}",
            nix_escape(&plan.workspace_root)
        )
    };

    out.push_str("{ pkgs ? import <nixpkgs> {}, src ? ");
    out.push_str(&default_src);
    out.push_str(", cargoHome ? null, gitSourceHashes ? {}, allowImpureGitFetch ? false, release ? ");
    out.push_str(release_default);
    out.push_str(" }:\n");

    out.push_str("let\n");
    out.push_str("  inherit (pkgs.lib) foldl';\n");
    out.push_str("  buildMode = if release then \"release\" else \"debug\";\n");
    let _ = writeln!(
        out,
        "  planWorkspaceRoot = \"{}\";",
        nix_escape(&plan.workspace_root)
    );
    let _ = writeln!(
        out,
        "  planTargetDir = \"{}\";",
        nix_escape(&plan.target_dir)
    );
    let _ = writeln!(
        out,
        "  planTargetTriple = {};",
        nix_optional_string(plan.target_triple.as_deref())
    );
    let _ = writeln!(out, "  markerSrc = \"{}\";", nix_escape(PATH_MARKER_SRC));
    let _ = writeln!(
        out,
        "  markerTarget = \"{}\";",
        nix_escape(PATH_MARKER_TARGET)
    );
    let _ = writeln!(
        out,
        "  markerCargoHome = \"{}\";",
        nix_escape(PATH_MARKER_CARGO_HOME)
    );
    let _ = writeln!(out, "  markerRustc = \"{}\";", nix_escape(PATH_MARKER_RUSTC));
    let _ = writeln!(
        out,
        "  markerCargoBin = \"{}\";",
        nix_escape(PATH_MARKER_CARGO_BIN)
    );

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
    let _ = writeln!(out, "  unsupportedCargoHomePackages = [ {} ];", unsupported_packages);
    out.push_str("  effectiveCargoHome =\n");
    out.push_str("    if cargoHome != null then cargoHome\n");
    out.push_str("    else if unsupportedCargoHomePackages == [ ] then materializedCargoHome\n");
    out.push_str("    else throw \"nix-cargo: cannot auto-materialize cargoHome for packages: ${builtins.concatStringsSep \", \" unsupportedCargoHomePackages}. Pass cargoHome override.\";\n");
    out.push_str("  emptySrc = pkgs.runCommand \"nix-cargo-empty-src\" {} ''\n");
    out.push_str("    mkdir -p \"$out\"\n");
    out.push_str("  '';\n");

    out.push_str("  cratePlan = [\n");
    for package in ordered_packages {
        let package_commands = commands_by_package
            .get(package.key.as_str())
            .cloned()
            .unwrap_or_default();
        let layout = package_layout.get(package.key.as_str());
        let target_triples = layout
            .map(|layout| layout.target_triples.clone())
            .unwrap_or_default();
        let needs_host_artifacts = layout.map(|layout| layout.needs_host_artifacts).unwrap_or(false);
        let package_source_prefixes = source_prefixes_by_package
            .get(package.key.as_str())
            .cloned()
            .unwrap_or_default();
        let command_script = render_command_script(&package_commands);
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

    out.push_str(
        "  workspacePackageKeys = map (p: p.key) (builtins.filter (p: p.workspaceMember) cratePlan);\n\n",
    );
    out.push_str("  packageDerivations = foldl' (acc: packageDef:\n");
    out.push_str("    let\n");
    out.push_str("      dependencyDrvs = map (key: builtins.getAttr key acc)\n");
    out.push_str(
        "        (builtins.filter (key: builtins.hasAttr key acc) packageDef.dependencies);\n",
    );
    out.push_str("      sourcePrefixes = packageDef.workspaceSourcePrefixes;\n");
    out.push_str("      packageSrc = if sourcePrefixes == [ ] then\n");
    out.push_str("        emptySrc\n");
    out.push_str("      else builtins.path {\n");
    out.push_str("        path = src;\n");
    out.push_str("        name = \"nix-cargo-src-${packageDef.name}-${packageDef.version}\";\n");
    out.push_str("        filter = path: pathType:\n");
    out.push_str("          let\n");
    out.push_str("            srcStr = toString src;\n");
    out.push_str("            relWithSlash = pkgs.lib.removePrefix srcStr (toString path);\n");
    out.push_str(
        "            rel = if relWithSlash == \"\" then \"\" else pkgs.lib.removePrefix \"/\" relWithSlash;\n",
    );
    out.push_str(
        "            keepWorkspaceRoot = rel == \"\" || rel == \"Cargo.toml\" || rel == \"Cargo.lock\" || rel == \".cargo\" || pkgs.lib.hasPrefix \".cargo/\" rel;\n",
    );
    out.push_str("            keepPackageSources = builtins.any (prefix:\n");
    out.push_str(
        "              prefix == \"\" || rel == prefix || pkgs.lib.hasPrefix (prefix + \"/\") rel\n",
    );
    out.push_str("            ) sourcePrefixes;\n");
    out.push_str("            keepAncestorDirs = pathType == \"directory\" && builtins.any (prefix:\n");
    out.push_str(
        "              rel == \"\" || prefix == \"\" || rel == prefix || pkgs.lib.hasPrefix (rel + \"/\") prefix\n",
    );
    out.push_str("            ) sourcePrefixes;\n");
    out.push_str("          in keepWorkspaceRoot || keepPackageSources || keepAncestorDirs;\n");
    out.push_str("      };\n");
    out.push_str("    in\n");
    out.push_str("    acc // {\n");
    out.push_str("      \"${packageDef.key}\" = pkgs.stdenv.mkDerivation {\n");
    out.push_str("        pname = \"cargo-${packageDef.name}\";\n");
    out.push_str("        version = packageDef.version;\n");
    out.push_str("        buildMode = buildMode;\n");
    out.push_str("        src = packageSrc;\n");
    out.push_str("        nativeBuildInputs = [ pkgs.rustc pkgs.cargo pkgs.pkg-config pkgs.stdenv.cc ];\n");
    out.push_str("        buildInputs = dependencyDrvs;\n");
    out.push_str("        dontFixup = true;\n");
    out.push_str("        doCheck = false;\n");
    out.push_str("        buildPhase = ''\n");
    out.push_str("          set -euo pipefail\n");
    out.push_str("          export CARGO_TARGET_DIR=\"$TMPDIR/target\"\n");
    out.push_str("          export NIXCARGO_SRC=\"${toString packageSrc}\"\n");
    out.push_str("          export NIXCARGO_CARGO_HOME=\"${toString effectiveCargoHome}\"\n");
    out.push_str("          export NIXCARGO_RUSTC=\"${pkgs.rustc}/bin/rustc\"\n");
    out.push_str("          export NIXCARGO_CARGO=\"${pkgs.cargo}/bin/cargo\"\n");
    out.push_str("          export CARGO_HOME=\"$NIXCARGO_CARGO_HOME\"\n");
    out.push_str("          markerTarget=\"${markerTarget}\"\n");
    out.push_str("          markerSrc=\"${markerSrc}\"\n");
    out.push_str("          markerCargoHome=\"${markerCargoHome}\"\n");
    out.push_str("          markerRustc=\"${markerRustc}\"\n");
    out.push_str("          markerCargoBin=\"${markerCargoBin}\"\n");
    out.push_str("          planWorkspaceRoot=\"${planWorkspaceRoot}\"\n");
    out.push_str("          planTargetDir=\"${planTargetDir}\"\n");
    out.push_str("          mkdir -p \"$CARGO_TARGET_DIR\" \"$CARGO_HOME\"\n");
    out.push_str("          mkdir -p \"$CARGO_TARGET_DIR/$buildMode\"\n");
    out.push_str("          mkdir -p \"$CARGO_TARGET_DIR/$buildMode/deps\"\n");
    out.push_str("          mkdir -p \"$CARGO_TARGET_DIR/$buildMode/build\"\n");
    out.push_str("          mkdir -p \"$CARGO_TARGET_DIR/$buildMode/examples\"\n");
    out.push_str("          depPaths=( ${pkgs.lib.escapeShellArgs (map toString dependencyDrvs)} )\n");
    out.push_str("          targetTriples=( ${pkgs.lib.escapeShellArgs packageDef.targetTriples} )\n");
    out.push_str("          needsHostArtifacts=${if packageDef.needsHostArtifacts then \"1\" else \"0\"}\n");
    out.push_str("          declare -A nixcargo_build_script_runs=()\n");
    out.push_str("          nixcargo_last_build_script_binary=\"\"\n");
    out.push_str("          for targetTriple in \"''${targetTriples[@]}\"; do\n");
    out.push_str("            mkdir -p \"$CARGO_TARGET_DIR/$targetTriple/$buildMode/deps\"\n");
    out.push_str("            mkdir -p \"$CARGO_TARGET_DIR/$targetTriple/$buildMode/build\"\n");
    out.push_str("            mkdir -p \"$CARGO_TARGET_DIR/$targetTriple/$buildMode/examples\"\n");
    out.push_str("          done\n");
    out.push_str("          copy_tree_if_exists() {\n");
    out.push_str("            local srcDir=\"$1\"\n");
    out.push_str("            local dstDir=\"$2\"\n");
    out.push_str("            if [ -d \"$srcDir\" ]; then\n");
    out.push_str("              mkdir -p \"$dstDir\"\n");
    out.push_str("              cp -R -n \"$srcDir/.\" \"$dstDir/\" || true\n");
    out.push_str("            fi\n");
    out.push_str("          }\n");
    out.push_str("          for depPath in \"''${depPaths[@]}\"; do\n");
    out.push_str("            if [ -d \"$depPath\" ]; then\n");
    out.push_str("              if [ \"$needsHostArtifacts\" -eq 1 ]; then\n");
    out.push_str("                copy_tree_if_exists \"$depPath/deps\" \"$CARGO_TARGET_DIR/$buildMode/deps\"\n");
    out.push_str("                copy_tree_if_exists \"$depPath/build\" \"$CARGO_TARGET_DIR/$buildMode/build\"\n");
    out.push_str("                copy_tree_if_exists \"$depPath/examples\" \"$CARGO_TARGET_DIR/$buildMode/examples\"\n");
    out.push_str("                copy_tree_if_exists \"$depPath/.fingerprint\" \"$CARGO_TARGET_DIR/$buildMode/.fingerprint\"\n");
    out.push_str("              fi\n");
    out.push_str("              for targetTriple in \"''${targetTriples[@]}\"; do\n");
    out.push_str("                copy_tree_if_exists \"$depPath/$targetTriple/deps\" \"$CARGO_TARGET_DIR/$targetTriple/$buildMode/deps\"\n");
    out.push_str("                copy_tree_if_exists \"$depPath/$targetTriple/build\" \"$CARGO_TARGET_DIR/$targetTriple/$buildMode/build\"\n");
    out.push_str("                copy_tree_if_exists \"$depPath/$targetTriple/examples\" \"$CARGO_TARGET_DIR/$targetTriple/$buildMode/examples\"\n");
    out.push_str("                copy_tree_if_exists \"$depPath/$targetTriple/.fingerprint\" \"$CARGO_TARGET_DIR/$targetTriple/$buildMode/.fingerprint\"\n");
    out.push_str("              done\n");
    out.push_str("            fi\n");
    out.push_str("          done\n");
    out.push_str("          rewrite_value() {\n");
    out.push_str("            local value=\"$1\"\n");
    out.push_str("            value=\"''${value//''${markerTarget}/$CARGO_TARGET_DIR}\"\n");
    out.push_str("            value=\"''${value//''${markerSrc}/$NIXCARGO_SRC}\"\n");
    out.push_str("            value=\"''${value//''${markerCargoHome}/$NIXCARGO_CARGO_HOME}\"\n");
    out.push_str("            value=\"''${value//''${markerRustc}/$NIXCARGO_RUSTC}\"\n");
    out.push_str("            value=\"''${value//''${markerCargoBin}/$NIXCARGO_CARGO}\"\n");
    out.push_str("            value=\"''${value//''${planWorkspaceRoot}/$NIXCARGO_SRC}\"\n");
    out.push_str("            value=\"''${value//''${planTargetDir}/$CARGO_TARGET_DIR}\"\n");
    out.push_str("            printf '%s' \"$value\"\n");
    out.push_str("          }\n");
    out.push_str("          run_cargo_cmd() {\n");
    out.push_str("            local cwdRaw=\"$1\"\n");
    out.push_str("            shift\n");
    out.push_str("            local programRaw=\"$1\"\n");
    out.push_str("            shift\n");
    out.push_str("            local -n argsRef=\"$1\"\n");
    out.push_str("            shift\n");
    out.push_str("            local -n envRef=\"$1\"\n");
    out.push_str("            local cwd program entry key value status outDir crateName outputPath commandOutDir\n");
    out.push_str("            local -a args=()\n");
    out.push_str("            local -a envArgs=()\n");
    out.push_str("            cwd=\"$(rewrite_value \"$cwdRaw\")\"\n");
    out.push_str("            program=\"$(rewrite_value \"$programRaw\")\"\n");
    out.push_str("            for entry in \"''${envRef[@]}\"; do\n");
    out.push_str("              key=\"''${entry%%=*}\"\n");
    out.push_str("              value=\"''${entry#*=}\"\n");
    out.push_str("              envArgs+=(\"''${key}=$(rewrite_value \"$value\")\")\n");
    out.push_str("            done\n");
    out.push_str("            local -a rewrittenArgs=()\n");
    out.push_str("            local nextIndex nextValue scanIndex emitSpec emitEntry\n");
    out.push_str("            for entry in \"''${argsRef[@]}\"; do\n");
    out.push_str("              rewrittenArgs+=(\"$(rewrite_value \"$entry\")\")\n");
    out.push_str("            done\n");
    out.push_str("            ensure_parent_dir() {\n");
    out.push_str("              local path=\"$1\"\n");
    out.push_str("              if [ -n \"$path\" ]; then\n");
    out.push_str("                mkdir -p \"$(dirname \"$path\")\"\n");
    out.push_str("              fi\n");
    out.push_str("            }\n");
    out.push_str("            ensure_dep_info_parent_dirs() {\n");
    out.push_str("              local emitSpecValue=\"$1\"\n");
    out.push_str("              local -a emitParts=()\n");
    out.push_str("              IFS=',' read -r -a emitParts <<< \"$emitSpecValue\"\n");
    out.push_str("              for emitEntry in \"''${emitParts[@]}\"; do\n");
    out.push_str("                if [[ \"$emitEntry\" == dep-info=* ]]; then\n");
    out.push_str("                  ensure_parent_dir \"''${emitEntry#dep-info=}\"\n");
    out.push_str("                fi\n");
    out.push_str("              done\n");
    out.push_str("            }\n");
    out.push_str("            scanIndex=0\n");
    out.push_str("            while [ \"$scanIndex\" -lt \"''${#rewrittenArgs[@]}\" ]; do\n");
    out.push_str("              entry=\"''${rewrittenArgs[$scanIndex]}\"\n");
    out.push_str("              if [ \"$entry\" = \"--out-dir\" ] && [ $((scanIndex + 1)) -lt \"''${#rewrittenArgs[@]}\" ]; then\n");
    out.push_str("                mkdir -p \"''${rewrittenArgs[$((scanIndex + 1))]}\"\n");
    out.push_str("              elif [[ \"$entry\" == --out-dir=* ]]; then\n");
    out.push_str("                mkdir -p \"''${entry#--out-dir=}\"\n");
    out.push_str("              elif [ \"$entry\" = \"-o\" ] && [ $((scanIndex + 1)) -lt \"''${#rewrittenArgs[@]}\" ]; then\n");
    out.push_str("                ensure_parent_dir \"''${rewrittenArgs[$((scanIndex + 1))]}\"\n");
    out.push_str("              elif [ \"$entry\" = \"--emit\" ] && [ $((scanIndex + 1)) -lt \"''${#rewrittenArgs[@]}\" ]; then\n");
    out.push_str("                ensure_dep_info_parent_dirs \"''${rewrittenArgs[$((scanIndex + 1))]}\"\n");
    out.push_str("              elif [[ \"$entry\" == --emit=* ]]; then\n");
    out.push_str("                ensure_dep_info_parent_dirs \"''${entry#--emit=}\"\n");
    out.push_str("              fi\n");
    out.push_str("              scanIndex=$((scanIndex + 1))\n");
    out.push_str("            done\n");
    out.push_str("            nextIndex=0\n");
    out.push_str("            while [ \"$nextIndex\" -lt \"''${#rewrittenArgs[@]}\" ]; do\n");
    out.push_str("              entry=\"''${rewrittenArgs[$nextIndex]}\"\n");
    out.push_str("              if [ \"$entry\" = \"-C\" ] && [ $((nextIndex + 1)) -lt \"''${#rewrittenArgs[@]}\" ]; then\n");
    out.push_str("                nextValue=\"''${rewrittenArgs[$((nextIndex + 1))]}\"\n");
    out.push_str("                if [[ \"$nextValue\" == incremental=* ]]; then\n");
    out.push_str("                  nextIndex=$((nextIndex + 2))\n");
    out.push_str("                  continue\n");
    out.push_str("                fi\n");
    out.push_str("              fi\n");
    out.push_str("              if [[ \"$entry\" == -Cincremental=* ]]; then\n");
    out.push_str("                nextIndex=$((nextIndex + 1))\n");
    out.push_str("                continue\n");
    out.push_str("              fi\n");
    out.push_str("              args+=(\"$entry\")\n");
    out.push_str("              nextIndex=$((nextIndex + 1))\n");
    out.push_str("            done\n");
    out.push_str("            outDir=\"\"\n");
    out.push_str("            for entry in \"''${envArgs[@]}\"; do\n");
    out.push_str("              key=\"''${entry%%=*}\"\n");
    out.push_str("              value=\"''${entry#*=}\"\n");
    out.push_str("              if [ \"$key\" = \"OUT_DIR\" ]; then\n");
    out.push_str("                outDir=\"$value\"\n");
    out.push_str("              fi\n");
    out.push_str("            done\n");
    out.push_str("            crateName=\"\"\n");
    out.push_str("            outputPath=\"\"\n");
    out.push_str("            commandOutDir=\"\"\n");
    out.push_str("            nextIndex=0\n");
    out.push_str("            while [ \"$nextIndex\" -lt \"''${#args[@]}\" ]; do\n");
    out.push_str("              entry=\"''${args[$nextIndex]}\"\n");
    out.push_str("              if [ \"$entry\" = \"--crate-name\" ] && [ $((nextIndex + 1)) -lt \"''${#args[@]}\" ]; then\n");
    out.push_str("                crateName=\"''${args[$((nextIndex + 1))]}\"\n");
    out.push_str("                nextIndex=$((nextIndex + 2))\n");
    out.push_str("                continue\n");
    out.push_str("              fi\n");
    out.push_str("              if [ \"$entry\" = \"-o\" ] && [ $((nextIndex + 1)) -lt \"''${#args[@]}\" ]; then\n");
    out.push_str("                outputPath=\"''${args[$((nextIndex + 1))]}\"\n");
    out.push_str("                nextIndex=$((nextIndex + 2))\n");
    out.push_str("                continue\n");
    out.push_str("              fi\n");
    out.push_str("              if [ \"$entry\" = \"--out-dir\" ] && [ $((nextIndex + 1)) -lt \"''${#args[@]}\" ]; then\n");
    out.push_str("                commandOutDir=\"''${args[$((nextIndex + 1))]}\"\n");
    out.push_str("                nextIndex=$((nextIndex + 2))\n");
    out.push_str("                continue\n");
    out.push_str("              fi\n");
    out.push_str("              if [[ \"$entry\" == --out-dir=* ]]; then\n");
    out.push_str("                commandOutDir=\"''${entry#--out-dir=}\"\n");
    out.push_str("                nextIndex=$((nextIndex + 1))\n");
    out.push_str("                continue\n");
    out.push_str("              fi\n");
    out.push_str("              nextIndex=$((nextIndex + 1))\n");
    out.push_str("            done\n");
    out.push_str("            if [ -n \"$outDir\" ] && [ -n \"''${nixcargo_last_build_script_binary:-}\" ] && [ -x \"''${nixcargo_last_build_script_binary}\" ] && [ \"''${crateName}\" != \"build_script_build\" ] && [ -z \"''${nixcargo_build_script_runs[$outDir]+x}\" ]; then\n");
    out.push_str("              mkdir -p \"$outDir\"\n");
    out.push_str("              env \"''${envArgs[@]}\" OUT_DIR=\"$outDir\" CARGO_MANIFEST_DIR=\"$cwd\" \"''${nixcargo_last_build_script_binary}\"\n");
    out.push_str("              nixcargo_build_script_runs[$outDir]=1\n");
    out.push_str("            fi\n");
    out.push_str("            if [ -n \"$cwd\" ]; then\n");
    out.push_str("              pushd \"$cwd\" > /dev/null\n");
    out.push_str("            fi\n");
    out.push_str("            env \"''${envArgs[@]}\" \"$program\" \"''${args[@]}\"\n");
    out.push_str("            status=$?\n");
    out.push_str("            if [ \"$status\" -eq 0 ] && [ \"''${crateName}\" = \"build_script_build\" ]; then\n");
    out.push_str("              if [ -z \"$outputPath\" ] && [ -n \"$commandOutDir\" ] && [ -d \"$commandOutDir\" ]; then\n");
    out.push_str("                outputPath=\"$(find \"$commandOutDir\" -maxdepth 1 -type f -name 'build_script_build*' -perm -u+x | head -n1 || true)\"\n");
    out.push_str("              fi\n");
    out.push_str("              if [ -n \"$outputPath\" ] && [ -x \"$outputPath\" ]; then\n");
    out.push_str("                nixcargo_last_build_script_binary=\"$outputPath\"\n");
    out.push_str("              fi\n");
    out.push_str("            fi\n");
    out.push_str("            if [ -n \"$cwd\" ]; then\n");
    out.push_str("              popd > /dev/null\n");
    out.push_str("            fi\n");
    out.push_str("            return \"$status\"\n");
    out.push_str("          }\n");
    out.push_str("          scriptFile=\"$TMPDIR/nix-cargo-package-commands.sh\"\n");
    out.push_str("          printf '%s\n' \"${packageDef.commandScript}\" > \"$scriptFile\"\n");
    out.push_str("          source \"$scriptFile\"\n");
    out.push_str("        '';\n");
    out.push_str("        installPhase = ''\n");
    out.push_str("          mkdir -p \"$out\"\n");
    out.push_str("          copied=0\n");
    out.push_str("          targetTriples=( ${pkgs.lib.escapeShellArgs packageDef.targetTriples} )\n");
    out.push_str("          needsHostArtifacts=${if packageDef.needsHostArtifacts then \"1\" else \"0\"}\n");
    out.push_str("          copy_install_layout() {\n");
    out.push_str("            local srcRoot=\"$1\"\n");
    out.push_str("            local dstRoot=\"$2\"\n");
    out.push_str("            if [ -d \"$srcRoot/deps\" ]; then\n");
    out.push_str("              mkdir -p \"$dstRoot/deps\"\n");
    out.push_str("              shopt -s nullglob\n");
    out.push_str("              for artifact in \"$srcRoot/deps\"/*; do\n");
    out.push_str("                case \"$artifact\" in\n");
    out.push_str("                  *.d) continue ;;\n");
    out.push_str("                esac\n");
    out.push_str("                cp -R \"$artifact\" \"$dstRoot/deps/\"\n");
    out.push_str("              done\n");
    out.push_str("              shopt -u nullglob\n");
    out.push_str("              copied=1\n");
    out.push_str("            fi\n");
    out.push_str("            if [ -d \"$srcRoot/build\" ]; then\n");
    out.push_str("              cp -R \"$srcRoot/build\" \"$dstRoot/\"\n");
    out.push_str("              copied=1\n");
    out.push_str("            fi\n");
    out.push_str("            if [ -d \"$srcRoot/examples\" ]; then\n");
    out.push_str("              cp -R \"$srcRoot/examples\" \"$dstRoot/\"\n");
    out.push_str("              copied=1\n");
    out.push_str("            fi\n");
    out.push_str("            if [ -d \"$srcRoot/.fingerprint\" ]; then\n");
    out.push_str("              cp -R \"$srcRoot/.fingerprint\" \"$dstRoot/\"\n");
    out.push_str("              copied=1\n");
    out.push_str("            fi\n");
    out.push_str("          }\n");
    out.push_str("          if [ \"$needsHostArtifacts\" -eq 1 ]; then\n");
    out.push_str("            copy_install_layout \"$CARGO_TARGET_DIR/$buildMode\" \"$out\"\n");
    out.push_str("          fi\n");
    out.push_str("          for targetTriple in \"''${targetTriples[@]}\"; do\n");
    out.push_str("            mkdir -p \"$out/$targetTriple\"\n");
    out.push_str("            copy_install_layout \"$CARGO_TARGET_DIR/$targetTriple/$buildMode\" \"$out/$targetTriple\"\n");
    out.push_str("          done\n");
    out.push_str("          if [ \"$copied\" -eq 0 ]; then\n");
    out.push_str("            touch \"$out/.nix-cargo-empty\"\n");
    out.push_str("          fi\n");
    out.push_str("        '';\n");
    out.push_str("      };\n");
    out.push_str("    }\n");
    out.push_str("  ) {} cratePlan;\n");
    out.push_str("  dynamicPackageDerivations = packageDerivations;\n");
    out.push_str(
        "  dynamicPackages = builtins.mapAttrs (_: drv: builtins.outputOf (builtins.unsafeDiscardOutputDependency drv.drvPath) \"out\") dynamicPackageDerivations;\n",
    );
    out.push_str("  packageLayouts = builtins.listToAttrs (map (packageDef: {\n");
    out.push_str("    name = packageDef.key;\n");
    out.push_str("    value = {\n");
    out.push_str("      targetTriples = packageDef.targetTriples;\n");
    out.push_str("      needsHostArtifacts = packageDef.needsHostArtifacts;\n");
    out.push_str("    };\n");
    out.push_str("  }) cratePlan);\n");
    out.push_str("  workspacePackageLayouts = builtins.listToAttrs (map (key: {\n");
    out.push_str("    name = key;\n");
    out.push_str("    value = builtins.getAttr key packageLayouts;\n");
    out.push_str("  }) workspacePackageKeys);\n");
    out.push_str("  workspaceDynamicPackages = builtins.listToAttrs (map (key: {\n");
    out.push_str("    name = key;\n");
    out.push_str("    value = builtins.getAttr key dynamicPackages;\n");
    out.push_str("  }) workspacePackageKeys);\n\n");
    out.push_str("in\n");
    out.push_str("{\n");
    out.push_str("  inherit packageDerivations dynamicPackageDerivations;\n");
    out.push_str("  packages = packageDerivations;\n");
    out.push_str("  dynamicPackages = dynamicPackages;\n");
    out.push_str("  targetTriple = planTargetTriple;\n");
    out.push_str("  packageLayouts = packageLayouts;\n");
    out.push_str("  workspacePackageLayouts = workspacePackageLayouts;\n");
    out.push_str("  driver = {\n");
    out.push_str("    kind = \"nix-cargo-driver\";\n");
    out.push_str("    targets = builtins.listToAttrs (map (key: {\n");
    out.push_str("      name = key;\n");
    out.push_str("      value = {\n");
    out.push_str("        ref = builtins.getAttr key dynamicPackageDerivations;\n");
    out.push_str("        target = builtins.getAttr key dynamicPackages;\n");
    out.push_str("      };\n");
    out.push_str("    }) (builtins.attrNames dynamicPackages));\n");
    out.push_str(
        "    workspaceTargets = builtins.listToAttrs (map (key: { name = key; value = builtins.getAttr key dynamicPackages; }) workspacePackageKeys);\n",
    );
    out.push_str(
        "    defaultWorkspaceTarget = if workspacePackageKeys == [ ] then null else builtins.getAttr (builtins.head workspacePackageKeys) dynamicPackages;\n",
    );
    out.push_str("  };\n");
    out.push_str("  workspacePackages = builtins.listToAttrs (map (key: {\n");
    out.push_str("    name = key;\n");
    out.push_str("    value = builtins.getAttr key packageDerivations;\n");
    out.push_str("  }) workspacePackageKeys);\n");
    out.push_str("  workspaceDynamicPackages = workspaceDynamicPackages;\n");
    out.push_str("  default = pkgs.buildEnv {\n");
    out.push_str("    name = \"nix-cargo-workspace\";\n");
    out.push_str("    paths = map (key: builtins.getAttr key packageDerivations) workspacePackageKeys;\n");
    out.push_str("    ignoreCollisions = true;\n");
    out.push_str("  };\n");
    out.push_str("}\n");

    out
}
