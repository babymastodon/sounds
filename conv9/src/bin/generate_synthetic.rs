use std::env;
use std::path::Path;

use anyhow::{Context, Result, bail};

fn main() -> Result<()> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments.len() != 2 {
        bail!("usage: generate_synthetic <source-id> <output.wav>");
    }
    conv9::synthetic::write_wav(&arguments[0], Path::new(&arguments[1]))
        .with_context(|| format!("generate synthetic source {}", arguments[0]))
}
