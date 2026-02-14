use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;

use crate::model::{
    CommandSpec, Plan, PlanPackage, PATH_MARKER_CARGO_BIN, PATH_MARKER_CARGO_HOME,
    PATH_MARKER_RUSTC, PATH_MARKER_SRC, PATH_MARKER_TARGET,
};
use crate::source_scope::workspace_source_prefixes_by_package;

pub fn render_nix_expression(plan: &Plan, release_mode: bool) -> String {
    let mut out = String::new();
    let ordered_packages = topologically_sorted_packages(plan);
    let commands_by_package = plan_commands_by_package(plan);
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

    for git_entry in &cargo_home_plan.git_crates {
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
        let package_source_prefixes = source_prefixes_by_package
            .get(package.key.as_str())
            .cloned()
            .unwrap_or_default();
        let command_script = render_command_script(&package_commands);
        let _ = writeln!(
            out,
            "    {{ key = \"{}\"; name = \"{}\"; version = \"{}\"; source = \"{}\"; lockChecksum = {}; cargoHomeRelManifestPath = {}; workspaceMember = {}; dependencies = {}; workspaceSourcePrefixes = {}; commandScript = \"{}\"; }}",
            nix_escape(&package.key),
            nix_escape(&package.name),
            nix_escape(&package.version),
            nix_escape(&package.source),
            nix_optional_string(package.lock_checksum.as_deref()),
            nix_optional_string(package.cargo_home_rel_manifest_path.as_deref()),
            nix_bool(package.workspace_member),
            nix_string_list(&package.dependencies),
            nix_string_list(&package_source_prefixes),
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
    out.push_str("          copy_tree_if_exists() {\n");
    out.push_str("            local srcDir=\"$1\"\n");
    out.push_str("            local dstDir=\"$2\"\n");
    out.push_str("            if [ -d \"$srcDir\" ]; then\n");
    out.push_str("              mkdir -p \"$dstDir\"\n");
    out.push_str("              cp -R -n \"$srcDir/.\" \"$dstDir/\" || true\n");
    out.push_str("            fi\n");
    out.push_str("          }\n");
    out.push_str("          shopt -s dotglob nullglob\n");
    out.push_str("          for depPath in \"''${depPaths[@]}\"; do\n");
    out.push_str("            if [ -d \"$depPath\" ]; then\n");
    out.push_str("              copy_tree_if_exists \"$depPath/deps\" \"$CARGO_TARGET_DIR/$buildMode/deps\"\n");
    out.push_str("              copy_tree_if_exists \"$depPath/build\" \"$CARGO_TARGET_DIR/$buildMode/build\"\n");
    out.push_str("              copy_tree_if_exists \"$depPath/examples\" \"$CARGO_TARGET_DIR/$buildMode/examples\"\n");
    out.push_str("              copy_tree_if_exists \"$depPath/.fingerprint\" \"$CARGO_TARGET_DIR/$buildMode/.fingerprint\"\n");
    out.push_str("              for entry in \"$depPath\"/*; do\n");
    out.push_str("                if [ -d \"$entry\" ]; then\n");
    out.push_str("                  baseName=\"$(basename \"$entry\")\"\n");
    out.push_str("                  case \"$baseName\" in deps|build|examples|.fingerprint|incremental) continue ;; esac\n");
    out.push_str("                  mkdir -p \"$CARGO_TARGET_DIR/$baseName\"\n");
    out.push_str("                  cp -R -n \"$entry/.\" \"$CARGO_TARGET_DIR/$baseName/\" || true\n");
    out.push_str("                fi\n");
    out.push_str("              done\n");
    out.push_str("            fi\n");
    out.push_str("          done\n");
    out.push_str("          shopt -u dotglob nullglob\n");
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
    out.push_str("            local cwd program entry key value status\n");
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
    out.push_str("            local nextIndex nextValue\n");
    out.push_str("            for entry in \"''${argsRef[@]}\"; do\n");
    out.push_str("              rewrittenArgs+=(\"$(rewrite_value \"$entry\")\")\n");
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
    out.push_str("            if [ -n \"$cwd\" ]; then\n");
    out.push_str("              pushd \"$cwd\" > /dev/null\n");
    out.push_str("            fi\n");
    out.push_str("            env \"''${envArgs[@]}\" \"$program\" \"''${args[@]}\"\n");
    out.push_str("            status=$?\n");
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
    out.push_str("          if [ -d \"$CARGO_TARGET_DIR/$buildMode/deps\" ]; then\n");
    out.push_str("            mkdir -p \"$out/deps\"\n");
    out.push_str("            shopt -s nullglob\n");
    out.push_str("            for artifact in \"$CARGO_TARGET_DIR/$buildMode/deps\"/*; do\n");
    out.push_str("              case \"$artifact\" in\n");
    out.push_str("                *.d) continue ;;\n");
    out.push_str("              esac\n");
    out.push_str("              cp -R \"$artifact\" \"$out/deps/\"\n");
    out.push_str("            done\n");
    out.push_str("            shopt -u nullglob\n");
    out.push_str("            copied=1\n");
    out.push_str("          fi\n");
    out.push_str("          if [ -d \"$CARGO_TARGET_DIR/$buildMode/build\" ]; then\n");
    out.push_str("            cp -R \"$CARGO_TARGET_DIR/$buildMode/build\" \"$out/\"\n");
    out.push_str("            copied=1\n");
    out.push_str("          fi\n");
    out.push_str("          if [ -d \"$CARGO_TARGET_DIR/$buildMode/examples\" ]; then\n");
    out.push_str("            cp -R \"$CARGO_TARGET_DIR/$buildMode/examples\" \"$out/\"\n");
    out.push_str("            copied=1\n");
    out.push_str("          fi\n");
    out.push_str("          if [ -d \"$CARGO_TARGET_DIR/$buildMode/.fingerprint\" ]; then\n");
    out.push_str("            cp -R \"$CARGO_TARGET_DIR/$buildMode/.fingerprint\" \"$out/\"\n");
    out.push_str("            copied=1\n");
    out.push_str("          fi\n");
    out.push_str("          shopt -s nullglob\n");
    out.push_str("          for targetDir in \"$CARGO_TARGET_DIR\"/*; do\n");
    out.push_str("            if [ -d \"$targetDir/$buildMode\" ]; then\n");
    out.push_str("              targetTriple=\"$(basename \"$targetDir\")\"\n");
    out.push_str("              mkdir -p \"$out/$targetTriple\"\n");
    out.push_str("              if [ -d \"$targetDir/$buildMode/deps\" ]; then\n");
    out.push_str("                mkdir -p \"$out/$targetTriple/deps\"\n");
    out.push_str("                for artifact in \"$targetDir/$buildMode/deps\"/*; do\n");
    out.push_str("                  case \"$artifact\" in\n");
    out.push_str("                    *.d) continue ;;\n");
    out.push_str("                  esac\n");
    out.push_str("                  cp -R \"$artifact\" \"$out/$targetTriple/deps/\"\n");
    out.push_str("                done\n");
    out.push_str("                copied=1\n");
    out.push_str("              fi\n");
    out.push_str("              if [ -d \"$targetDir/$buildMode/build\" ]; then\n");
    out.push_str("                cp -R \"$targetDir/$buildMode/build\" \"$out/$targetTriple/\"\n");
    out.push_str("                copied=1\n");
    out.push_str("              fi\n");
    out.push_str("              if [ -d \"$targetDir/$buildMode/examples\" ]; then\n");
    out.push_str("                cp -R \"$targetDir/$buildMode/examples\" \"$out/$targetTriple/\"\n");
    out.push_str("                copied=1\n");
    out.push_str("              fi\n");
    out.push_str("              if [ -d \"$targetDir/$buildMode/.fingerprint\" ]; then\n");
    out.push_str("                cp -R \"$targetDir/$buildMode/.fingerprint\" \"$out/$targetTriple/\"\n");
    out.push_str("                copied=1\n");
    out.push_str("              fi\n");
    out.push_str("            fi\n");
    out.push_str("          done\n");
    out.push_str("          shopt -u nullglob\n");
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
    out.push_str("  workspaceDynamicPackages = builtins.listToAttrs (map (key: {\n");
    out.push_str("    name = key;\n");
    out.push_str("    value = builtins.getAttr key dynamicPackages;\n");
    out.push_str("  }) workspacePackageKeys);\n\n");
    out.push_str("in\n");
    out.push_str("{\n");
    out.push_str("  inherit packageDerivations dynamicPackageDerivations;\n");
    out.push_str("  packages = packageDerivations;\n");
    out.push_str("  dynamicPackages = dynamicPackages;\n");
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

#[derive(Debug, Clone)]
struct CargoHomeMaterializationPlan {
    registry_crates: Vec<RegistryCrateMaterialization>,
    git_crates: Vec<GitCrateMaterialization>,
    unsupported_package_keys: Vec<String>,
}

#[derive(Debug, Clone)]
struct RegistryCrateMaterialization {
    archive_binding: String,
    registry_src_parent: String,
    download_url: String,
    hash_sri: String,
}

#[derive(Debug, Clone)]
struct GitCrateMaterialization {
    source_binding: String,
    source_key: String,
    url: String,
    rev: String,
    destination_parent_rel: String,
    repo_subpath: Option<String>,
}

fn build_cargo_home_materialization_plan(plan: &Plan) -> CargoHomeMaterializationPlan {
    let mut registry_crates = Vec::new();
    let mut git_crates = Vec::new();
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

            let binding = format!("cargoGitSource{}", git_crates.len());
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

fn plan_commands_by_package(plan: &Plan) -> HashMap<String, Vec<CommandSpec>> {
    plan.packages
        .iter()
        .map(|package| {
            let commands = plan
                .units
                .iter()
                .filter(|unit| unit.package_key == package.key)
                .map(|unit| unit.command.clone())
                .collect::<Vec<_>>();
            (package.key.clone(), commands)
        })
        .collect()
}

fn topologically_sorted_packages(plan: &Plan) -> Vec<&PlanPackage> {
    fn visit<'a>(
        index: usize,
        plan: &'a Plan,
        key_to_index: &HashMap<&'a str, usize>,
        marks: &mut [u8],
        out: &mut Vec<&'a PlanPackage>,
    ) {
        if marks[index] == 2 {
            return;
        }
        if marks[index] == 1 {
            return;
        }

        marks[index] = 1;
        for dependency in &plan.packages[index].dependencies {
            if let Some(&dep_index) = key_to_index.get(dependency.as_str()) {
                visit(dep_index, plan, key_to_index, marks, out);
            }
        }
        marks[index] = 2;
        out.push(&plan.packages[index]);
    }

    let key_to_index = plan
        .packages
        .iter()
        .enumerate()
        .map(|(index, package)| (package.key.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut marks = vec![0u8; plan.packages.len()];
    let mut out = Vec::with_capacity(plan.packages.len());

    for index in 0..plan.packages.len() {
        visit(index, plan, &key_to_index, &mut marks, &mut out);
    }

    out
}

fn render_command_script(commands: &[CommandSpec]) -> String {
    let mut script = String::new();

    for (index, command) in commands.iter().enumerate() {
        let args_var = format!("cmd_args_{index}");
        let env_var = format!("cmd_env_{index}");
        let cwd = command.cwd.clone().unwrap_or_default();
        let env_pairs = command
            .env
            .iter()
            .map(|entry| format!("{}={}", entry.key, entry.value))
            .collect::<Vec<_>>();

        script.push_str("{\n");
        let _ = writeln!(
            script,
            "  declare -a {args_var}=({})",
            shell_array_literal(&command.args)
        );
        let _ = writeln!(
            script,
            "  declare -a {env_var}=({})",
            shell_array_literal(&env_pairs)
        );
        let _ = writeln!(
            script,
            "  run_cargo_cmd {} {} {args_var} {env_var}",
            shell_single_quote(&cwd),
            shell_single_quote(&command.program),
        );
        let _ = writeln!(script, "  unset {args_var} {env_var}");
        script.push_str("}\n");
    }

    script
}

fn shell_array_literal(values: &[String]) -> String {
    if values.is_empty() {
        return String::new();
    }

    values
        .iter()
        .map(|value| shell_single_quote(value))
        .collect::<Vec<_>>()
        .join(" ")
}

fn nix_string_list(values: &[String]) -> String {
    if values.is_empty() {
        return String::from("[ ]");
    }

    let mut result = String::from("[");
    for value in values {
        let _ = write!(result, " \"{}\"", nix_escape(value));
    }
    result.push_str(" ]");
    result
}

fn nix_optional_string(value: Option<&str>) -> String {
    value
        .map(|value| format!("\"{}\"", nix_escape(value)))
        .unwrap_or_else(|| String::from("null"))
}

fn nix_bool(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn nix_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('\n', "\\n")
}
