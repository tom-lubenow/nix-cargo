mod cargo_plan;
mod cargo_home;
mod command_script;
mod command_layout;
mod model;
mod nix_emit;
mod nix_string;
mod source_scope;
mod workspace;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use serde_json::to_string_pretty;
use workspace::summarize_workspace;

use model::{CommandSpec, WorkspaceSummary};

#[derive(Parser)]
#[command(name = "nix-cargo")]
#[command(about = "Prototype for Rust crate-level Nix graph generation", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print workspace package graph.
    Graph {
        #[arg(short, long)]
        manifest_path: Option<PathBuf>,
        #[arg(short, long)]
        json: bool,
    },
    /// Print package-level build plan and captured Cargo executor units.
    Plan {
        #[arg(short, long)]
        manifest_path: Option<PathBuf>,
        #[arg(short, long)]
        json: bool,
        /// Emit release build command shape.
        #[arg(long)]
        release: bool,
        /// Build for this target triple (plumbed to Cargo build requested_kinds).
        #[arg(long)]
        target_triple: Option<String>,
    },
    /// Emit a Nix expression with per-package derivations plus dynamic output refs.
    Emit {
        #[arg(short, long)]
        manifest_path: Option<PathBuf>,
        /// Write output to a file instead of stdout.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Use --release for generated crate build commands.
        #[arg(long)]
        release: bool,
        /// Build for this target triple (plumbed to Cargo build requested_kinds).
        #[arg(long)]
        target_triple: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Graph {
            manifest_path,
            json,
        } => {
            let summary = summarize_workspace(manifest_path.as_deref())?;
            if json {
                print_json(&summary)?;
            } else {
                print_graph(&summary);
            }
        }
        Commands::Plan {
            manifest_path,
            json,
            release,
            target_triple,
        } => {
            let plan = cargo_plan::build_plan(
                manifest_path.as_deref(),
                release,
                target_triple.as_deref(),
            )?;

            if json {
                print_json(&plan)?;
            } else {
                for unit in &plan.units {
                    println!("{}", unit.unit_id);
                    println!("  package: {} ({})", unit.package_name, unit.package_key);
                    println!("  target: {} ({})", unit.target_name, unit.target_kind);
                    println!("  mode: {}", unit.compile_mode);
                    println!(
                        "  command: {}",
                        render_command_for_display(&unit.command)
                    );
                    println!("  package deps: {}", unit.package_dependencies.join(", "));
                }
            }
        }
        Commands::Emit {
            manifest_path,
            output,
            release,
            target_triple,
        } => {
            let plan = cargo_plan::build_plan(
                manifest_path.as_deref(),
                release,
                target_triple.as_deref(),
            )?;
            let generated = nix_emit::render_nix_expression(&plan, release);

            match output {
                Some(path) => {
                    std::fs::write(&path, generated)
                        .with_context(|| format!("failed to write nix output to {}", path.display()))?;
                    println!("generated: {}", path.display());
                }
                None => print!("{generated}"),
            }
        }
    }

    Ok(())
}

fn print_graph(summary: &WorkspaceSummary) {
    println!("workspace: {}", summary.workspace_root);
    println!("manifest: {}", summary.manifest_path);
    println!("packages:");
    for package in &summary.packages {
        println!("  {} ({})", package.name, package.version);
        if !package.workspace_dependencies.is_empty() {
            println!("    workspace deps: {}", package.workspace_dependencies.join(", "));
        }
        if !package.dependency_names.is_empty() {
            println!("    all deps: {}", package.dependency_names.join(", "));
        }
    }
}

fn print_json<T>(value: &T) -> Result<()>
where
    T: serde::Serialize,
{
    let formatted = to_string_pretty(value)?;
    println!("{formatted}");
    Ok(())
}

fn render_command_for_display(command: &CommandSpec) -> String {
    let mut parts = Vec::new();
    if let Some(cwd) = &command.cwd {
        parts.push(format!("cd {} &&", shell_escape(cwd)));
    }
    for env in &command.env {
        parts.push(format!(
            "{}={}",
            shell_escape(&env.key),
            shell_escape(&env.value)
        ));
    }
    parts.push(shell_escape(&command.program));
    for arg in &command.args {
        parts.push(shell_escape(arg));
    }
    parts.join(" ")
}

fn shell_escape(value: &str) -> String {
    if value.is_empty() {
        return String::from("''");
    }
    if value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-_./:@=+".contains(&b))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
