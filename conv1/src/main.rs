use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use conv1::{
    ConcatOptions, RenderOptions, VerifyOptions, concatenate_master, render_matrix, verify_matrix,
};

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
    /// Concatenate every canonical WAV into FLAC and AAC masters.
    Concat {
        #[arg(long, default_value = "sources.tsv")]
        manifest: PathBuf,
        #[arg(long, default_value = "outputs/metrics.csv")]
        metrics: PathBuf,
        #[arg(long, default_value = "outputs/final")]
        output_dir: PathBuf,
        #[arg(long, default_value_t = 5.0)]
        crossfade_seconds: f64,
        #[arg(long, default_value_t = 192)]
        aac_bitrate_kbps: u32,
        #[arg(long, default_value_t = 64)]
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
        Command::Concat {
            manifest,
            metrics,
            output_dir,
            crossfade_seconds,
            aac_bitrate_kbps,
            opus_bitrate_kbps,
            force,
        } => concatenate_master(ConcatOptions {
            manifest,
            metrics,
            output_dir,
            crossfade_seconds,
            aac_bitrate_kbps,
            opus_bitrate_kbps,
            force,
        }),
    }
}

fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
}
