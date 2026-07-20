use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use conv8::{
    ConcatOptions, PitchApproach, RenderOptions, VerifyOptions, concatenate_master, render_matrix,
    verify_matrix,
};

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Render both additive-note approaches over the 24x24 bipartite matrix.
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
    /// Exhaustively validate both 24x24 matrix outputs.
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
    /// Concatenate each pitch approach into its own set of final masters.
    Concat {
        #[arg(long, default_value = "sources.tsv")]
        manifest: PathBuf,
        #[arg(long, default_value = "outputs")]
        matrix_dir: PathBuf,
        #[arg(long, default_value = "outputs/final")]
        output_dir: PathBuf,
        #[arg(long, default_value_t = 10.0)]
        crossfade_seconds: f64,
        #[arg(long, default_value_t = 192)]
        aac_bitrate_kbps: u32,
        #[arg(long, default_value_t = 128)]
        opus_bitrate_kbps: u32,
        /// Rebuild the RF64 and all final encodings even if they already exist.
        #[arg(long)]
        force: bool,
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
        } => {
            let jobs = jobs.unwrap_or_else(default_jobs);
            for approach in PitchApproach::ALL {
                render_matrix(RenderOptions {
                    manifest: manifest.clone(),
                    input_dir: input_dir.clone(),
                    output_dir: output_dir.join(approach.slug()),
                    jobs,
                    force,
                    approach,
                })?;
            }
            Ok(())
        }
        Command::Verify {
            manifest,
            input_dir,
            output_dir,
            jobs,
        } => {
            let jobs = jobs.unwrap_or_else(default_jobs);
            for approach in PitchApproach::ALL {
                verify_matrix(VerifyOptions {
                    manifest: manifest.clone(),
                    input_dir: input_dir.clone(),
                    output_dir: output_dir.join(approach.slug()),
                    jobs,
                    approach,
                })?;
            }
            Ok(())
        }
        Command::Concat {
            manifest,
            matrix_dir,
            output_dir,
            crossfade_seconds,
            aac_bitrate_kbps,
            opus_bitrate_kbps,
            force,
        } => {
            for approach in PitchApproach::ALL {
                concatenate_master(ConcatOptions {
                    manifest: manifest.clone(),
                    metrics: matrix_dir.join(approach.slug()).join("metrics.csv"),
                    output_dir: output_dir.join(approach.slug()),
                    crossfade_seconds,
                    aac_bitrate_kbps,
                    opus_bitrate_kbps,
                    force,
                })?;
            }
            Ok(())
        }
    }
}

fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
}
