use std::fmt::Write;

use crate::model::{
    Plan, PATH_MARKER_CARGO_BIN, PATH_MARKER_CARGO_HOME, PATH_MARKER_RUSTC, PATH_MARKER_SRC,
    PATH_MARKER_TARGET,
};
use crate::nix_string::{nix_escape, nix_optional_string};

pub(crate) fn append_preamble(
    out: &mut String,
    plan: &Plan,
    default_src: &str,
    release_default: &str,
) {
    out.push_str("{ pkgs ? import <nixpkgs> {}, src ? ");
    out.push_str(default_src);
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
}

