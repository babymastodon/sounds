use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use conv10::{
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
    /// Render the long-additive-synth short-to-long convolution matrix.
    Render {
        #[arg(long, default_value = "sources.tsv")]
        manifest: PathBuf,
        #[arg(long, default_value = "samples/prepared")]
        input_dir: PathBuf,
        #[arg(long, default_value = "outputs/long_additive_synth")]
        output_dir: PathBuf,
        /// Number of pair renders to run concurrently. Defaults to all logical CPUs.
        #[arg(long)]
        jobs: Option<usize>,
        /// Re-render WAVs that already exist instead of validating and reusing them.
        #[arg(long)]
        force: bool,
    },
    /// Exhaustively validate the long-additive-synth matrix.
    Verify {
        #[arg(long, default_value = "sources.tsv")]
        manifest: PathBuf,
        #[arg(long, default_value = "samples/prepared")]
        input_dir: PathBuf,
        #[arg(long, default_value = "outputs/long_additive_synth")]
        output_dir: PathBuf,
        #[arg(long)]
        jobs: Option<usize>,
    },
    /// Concatenate the matrix into same-named FLAC, AAC/M4A, and Opus masters.
    Concat {
        #[arg(long, default_value = "sources.tsv")]
        manifest: PathBuf,
        #[arg(long, default_value = "outputs/long_additive_synth")]
        matrix_dir: PathBuf,
        #[arg(long, default_value = "outputs/final")]
        output_dir: PathBuf,
        #[arg(long, default_value = ".scratch/concat")]
        scratch_dir: PathBuf,
        #[arg(long, default_value = "final_mix")]
        output_name: String,
        #[arg(long, default_value_t = 10.0)]
        crossfade_seconds: f64,
        #[arg(long, default_value_t = 192)]
        aac_bitrate_kbps: u32,
        #[arg(long, default_value_t = 128)]
        opus_bitrate_kbps: u32,
        /// Rebuild all three final encodings even if they already exist.
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
            render_matrix(RenderOptions {
                manifest,
                input_dir,
                output_dir,
                jobs,
                force,
                approach: PitchApproach::LongAdditiveSynth,
            })
        }
        Command::Verify {
            manifest,
            input_dir,
            output_dir,
            jobs,
        } => {
            let jobs = jobs.unwrap_or_else(default_jobs);
            verify_matrix(VerifyOptions {
                manifest,
                input_dir,
                output_dir,
                jobs,
                approach: PitchApproach::LongAdditiveSynth,
            })
        }
        Command::Concat {
            manifest,
            matrix_dir,
            output_dir,
            scratch_dir,
            output_name,
            crossfade_seconds,
            aac_bitrate_kbps,
            opus_bitrate_kbps,
            force,
        } => concatenate_master(ConcatOptions {
            manifest,
            metrics: matrix_dir.join("metrics.csv"),
            output_dir,
            scratch_dir,
            output_name,
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
