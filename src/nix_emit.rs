use std::fmt::Write;

use crate::cargo_home::build_cargo_home_materialization_plan;
use crate::command_script::render_command_script;
use crate::command_layout::package_layout_by_key;
use crate::model::Plan;
use crate::nix_string::{
    nix_bool, nix_escape, nix_optional_string, nix_string_list,
};
use crate::nix_cargo_home_emit::append_cargo_home_section;
use crate::nix_header_emit::append_preamble;
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

    out.push_str("  cratePlan = [\n");
    for package in ordered_packages {
        let package_units = units_by_package
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
        let command_script = render_command_script(&package_units);
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
    out.push_str("          declare -A nixcargo_build_script_binaries=()\n");
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
    out.push_str("              cp -R -n \"$srcDir/.\" \"$dstDir/\"\n");
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
    out.push_str("            local isBuildScriptCompile=\"$1\"\n");
    out.push_str("            shift\n");
    out.push_str("            local buildScriptBinaryHintRaw=\"$1\"\n");
    out.push_str("            shift\n");
    out.push_str("            local cwdRaw=\"$1\"\n");
    out.push_str("            shift\n");
    out.push_str("            local programRaw=\"$1\"\n");
    out.push_str("            shift\n");
    out.push_str("            local -n argsRef=\"$1\"\n");
    out.push_str("            shift\n");
    out.push_str("            local -n envRef=\"$1\"\n");
    out.push_str("            local cwd program entry key value status outDir manifestDir runDir outputPath commandOutDir buildScriptBinary buildScriptBinaryHint\n");
    out.push_str("            local -a args=()\n");
    out.push_str("            local -a envArgs=()\n");
    out.push_str("            cwd=\"$(rewrite_value \"$cwdRaw\")\"\n");
    out.push_str("            program=\"$(rewrite_value \"$programRaw\")\"\n");
    out.push_str("            buildScriptBinaryHint=\"$(rewrite_value \"$buildScriptBinaryHintRaw\")\"\n");
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
    out.push_str("            manifestDir=\"\"\n");
    out.push_str("            for entry in \"''${envArgs[@]}\"; do\n");
    out.push_str("              key=\"''${entry%%=*}\"\n");
    out.push_str("              value=\"''${entry#*=}\"\n");
    out.push_str("              if [ \"$key\" = \"OUT_DIR\" ]; then\n");
    out.push_str("                outDir=\"$value\"\n");
    out.push_str("              fi\n");
    out.push_str("              if [ \"$key\" = \"CARGO_MANIFEST_DIR\" ]; then\n");
    out.push_str("                manifestDir=\"$value\"\n");
    out.push_str("              fi\n");
    out.push_str("            done\n");
    out.push_str("            runDir=\"$manifestDir\"\n");
    out.push_str("            if [ -z \"$runDir\" ]; then\n");
    out.push_str("              runDir=\"$cwd\"\n");
    out.push_str("            fi\n");
    out.push_str("            outputPath=\"\"\n");
    out.push_str("            commandOutDir=\"\"\n");
    out.push_str("            nextIndex=0\n");
    out.push_str("            while [ \"$nextIndex\" -lt \"''${#args[@]}\" ]; do\n");
    out.push_str("              entry=\"''${args[$nextIndex]}\"\n");
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
    out.push_str("            buildScriptBinary=\"\"\n");
    out.push_str("            if [ -n \"$buildScriptBinaryHint\" ]; then\n");
    out.push_str("              buildScriptBinary=\"$buildScriptBinaryHint\"\n");
    out.push_str("            elif [ -n \"$runDir\" ] && [ -n \"''${nixcargo_build_script_binaries[$runDir]:-}\" ]; then\n");
    out.push_str("              buildScriptBinary=\"''${nixcargo_build_script_binaries[$runDir]}\"\n");
    out.push_str("            fi\n");
    out.push_str("            if [ \"$isBuildScriptCompile\" -ne 1 ] && [ -n \"$outDir\" ] && [ -n \"$buildScriptBinary\" ] && [ -x \"$buildScriptBinary\" ] && [ -z \"''${nixcargo_build_script_runs[$outDir]+x}\" ]; then\n");
    out.push_str("              mkdir -p \"$outDir\"\n");
    out.push_str("              if [ -n \"$runDir\" ]; then\n");
    out.push_str("                (cd \"$runDir\" && env \"''${envArgs[@]}\" OUT_DIR=\"$outDir\" CARGO_MANIFEST_DIR=\"$runDir\" \"$buildScriptBinary\")\n");
    out.push_str("              else\n");
    out.push_str("                env \"''${envArgs[@]}\" OUT_DIR=\"$outDir\" CARGO_MANIFEST_DIR=\"$runDir\" \"$buildScriptBinary\"\n");
    out.push_str("              fi\n");
    out.push_str("              nixcargo_build_script_runs[$outDir]=1\n");
    out.push_str("            fi\n");
    out.push_str("            if [ -n \"$cwd\" ]; then\n");
    out.push_str("              pushd \"$cwd\" > /dev/null\n");
    out.push_str("            fi\n");
    out.push_str("            env \"''${envArgs[@]}\" \"$program\" \"''${args[@]}\"\n");
    out.push_str("            status=$?\n");
    out.push_str("            if [ \"$status\" -eq 0 ] && [ \"$isBuildScriptCompile\" -eq 1 ]; then\n");
    out.push_str("              if [ -z \"$outputPath\" ] && [ -n \"$buildScriptBinaryHint\" ]; then\n");
    out.push_str("                outputPath=\"$buildScriptBinaryHint\"\n");
    out.push_str("              fi\n");
    out.push_str("              if [ -z \"$outputPath\" ] && [ -n \"$commandOutDir\" ] && [ -d \"$commandOutDir\" ]; then\n");
    out.push_str("                outputPath=\"$(find \"$commandOutDir\" -maxdepth 1 -type f -name 'build_script_build*' -perm -u+x | LC_ALL=C sort | head -n1 || true)\"\n");
    out.push_str("              fi\n");
    out.push_str("              if [ -n \"$outputPath\" ] && [ -x \"$outputPath\" ]; then\n");
    out.push_str("                if [ -n \"$runDir\" ]; then\n");
    out.push_str("                  nixcargo_build_script_binaries[$runDir]=\"$outputPath\"\n");
    out.push_str("                fi\n");
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
    append_public_attrs_section(&mut out);

    out
}
