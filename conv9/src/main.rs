use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};

use conv9::{Algorithm, RenderOptions, VerifyOptions, WindowPreset, render_matrix, verify_matrix};

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Render all selected pair/algorithm/window combinations.
    Render {
        #[arg(long, default_value = "sources.tsv")]
        manifest: PathBuf,
        #[arg(long, default_value = "samples/prepared")]
        input_dir: PathBuf,
        #[arg(long, default_value = "outputs")]
        output_dir: PathBuf,
        /// Number of independent output renders to run concurrently.
        #[arg(long)]
        jobs: Option<usize>,
        /// Re-render valid files that already exist.
        #[arg(long)]
        force: bool,
        /// Restrict to one of: multiresolution, sliding_wola, evolving_ir, chunk_crossfade.
        #[arg(long)]
        algorithm: Option<Algorithm>,
        /// Restrict to one of: short, medium, long.
        #[arg(long)]
        preset: Option<WindowPreset>,
        /// Restrict to one canonical pair slug: first_id__second_id.
        #[arg(long)]
        pair: Option<String>,
    },
    /// Exhaustively verify the complete 792-file output matrix.
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
            algorithm,
            preset,
            pair,
        } => {
            configure_threads(jobs)?;
            render_matrix(RenderOptions {
                manifest,
                input_dir,
                output_dir,
                force,
                algorithm,
                preset,
                pair,
            })
        }
        Command::Verify {
            manifest,
            input_dir,
            output_dir,
            jobs,
        } => {
            configure_threads(jobs)?;
            verify_matrix(VerifyOptions {
                manifest,
                input_dir,
                output_dir,
            })
        }
    }
}

fn configure_threads(jobs: Option<usize>) -> Result<()> {
    let jobs = jobs.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            .min(4)
    });
    if jobs == 0 {
        bail!("--jobs must be at least 1");
    }
    rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build_global()?;
    Ok(())
}
