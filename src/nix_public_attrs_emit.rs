pub(crate) fn append_public_attrs_section(out: &mut String) {
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
}

