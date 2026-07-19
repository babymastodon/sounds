use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use conv1::{RenderOptions, VerifyOptions, render_matrix, verify_matrix};

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Render every unique pair, including self-convolutions.
    Render {
        #[arg(long, default_value = "sources.tsv")]
        manifest: PathBuf,
        #[arg(long, default_value = "samples/prepared")]
        input_dir: PathBuf,
        #[arg(long, default_value = "outputs")]
        output_dir: PathBuf,
        /// Number of pair renders to run concurrently. Defaults to all logical CPUs.
        #[arg(long)]
        jobs: Option<usize>,
        /// Re-render WAVs that already exist instead of validating and reusing them.
        #[arg(long)]
        force: bool,
    },
    /// Exhaustively validate all expected matrix outputs.
    Verify {
        #[arg(long, default_value = "sources.tsv")]
        manifest: PathBuf,
        #[arg(long, default_value = "samples/prepared")]
        input_dir: PathBuf,
        #[arg(long, default_value = "outputs")]
        output_dir: PathBuf,
        #[arg(long)]
        jobs: Option<usize>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Render {
            manifest,
            input_dir,
            output_dir,
            jobs,
            force,
        } => render_matrix(RenderOptions {
            manifest,
            input_dir,
            output_dir,
            jobs: jobs.unwrap_or_else(default_jobs),
            force,
        }),
        Command::Verify {
            manifest,
            input_dir,
            output_dir,
            jobs,
        } => verify_matrix(VerifyOptions {
            manifest,
            input_dir,
            output_dir,
            jobs: jobs.unwrap_or_else(default_jobs),
        }),
    }
}

fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
}
