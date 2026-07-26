use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use conv9::{AlgorithmParameters, OnDemandRenderer, RenderSelection};

fn main() -> Result<()> {
    let mut arguments = std::env::args().skip(1);
    let a_seconds = parse_or(arguments.next(), 0.10)?;
    let b_seconds = parse_or(arguments.next(), 5.00)?;
    let overlap_percent = parse_or(arguments.next(), 75.0)?;
    let repeats = parse_or(arguments.next(), 1_usize)?;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let renderer =
        OnDemandRenderer::load(&root.join("sources.tsv"), &root.join("samples/prepared"))?;
    let mut windows = HashMap::new();
    windows.insert("clip_a_seconds".to_owned(), a_seconds);
    windows.insert("clip_b_seconds".to_owned(), b_seconds);
    let selection = RenderSelection {
        left_id: "ocean_rocks".to_owned(),
        right_id: "drava_river_rapids".to_owned(),
        algorithm: "windowed_convolution".to_owned(),
        windows,
        parameters: AlgorithmParameters {
            window_overlap_percent: overlap_percent,
            ..AlgorithmParameters::default()
        },
    };

    for repeat in 0..repeats {
        let started = Instant::now();
        let rendered = renderer.render(&selection, &|| false)?;
        println!(
            "run={} windows={a_seconds:.2}x{b_seconds:.2}s overlap={overlap_percent:.1}% \
             threads={} dsp_ms={:.1} total_ms={:.1} wall_ms={:.1} frames={} rms={:.6} \
             wav_fnv64={:016x}",
            repeat + 1,
            rayon::current_num_threads(),
            rendered.timings.dsp_milliseconds,
            rendered.timings.total_milliseconds,
            started.elapsed().as_secs_f64() * 1_000.0,
            rendered.metrics.frames,
            rendered.metrics.rms,
            fnv1a64(&rendered.wav),
        );
    }
    Ok(())
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn parse_or<T>(argument: Option<String>, default: T) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    argument
        .map(|value| value.parse().context("parse benchmark argument"))
        .transpose()
        .map(|value| value.unwrap_or(default))
}
