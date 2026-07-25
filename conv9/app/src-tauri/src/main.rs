use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use conv9::{AlgorithmParameters, Catalog, OnDemandRenderer, RenderSelection};
use serde::Serialize;
use tauri::Manager;
use tauri::ipc::Response;

const ENVELOPE_MAGIC: &[u8; 4] = b"CV9R";

struct AppState {
    renderer: Arc<OnDemandRenderer>,
    render_lock: Arc<Mutex<()>>,
    latest_request: Arc<AtomicU64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Bootstrap {
    catalog: Catalog,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RenderHeader {
    request_id: u64,
    left_id: String,
    right_id: String,
    algorithm: String,
    windows: HashMap<String, f32>,
    hop_seconds: Option<f32>,
    render_milliseconds: u128,
    metrics: conv9::AudioMetrics,
}

#[tauri::command]
fn load_bootstrap(state: tauri::State<'_, AppState>) -> Bootstrap {
    Bootstrap {
        catalog: state.renderer.catalog(),
    }
}

#[tauri::command]
async fn render_selection(
    state: tauri::State<'_, AppState>,
    request_id: u64,
    left_id: String,
    right_id: String,
    algorithm: String,
    windows: HashMap<String, f32>,
    parameters: AlgorithmParameters,
) -> Result<Response, String> {
    state.latest_request.fetch_max(request_id, Ordering::AcqRel);
    let renderer = Arc::clone(&state.renderer);
    let render_lock = Arc::clone(&state.render_lock);
    let latest_request = Arc::clone(&state.latest_request);
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = render_lock
            .lock()
            .map_err(|_| "render coordinator lock was poisoned".to_owned())?;
        let cancelled = || latest_request.load(Ordering::Acquire) != request_id;
        if cancelled() {
            return Err("render superseded".to_owned());
        }
        let started = Instant::now();
        let selection = RenderSelection {
            left_id: left_id.clone(),
            right_id: right_id.clone(),
            algorithm: algorithm.clone(),
            windows,
            parameters,
        };
        let rendered = renderer
            .render(&selection, &cancelled)
            .map_err(|error| error.to_string())?;
        if cancelled() {
            return Err("render superseded".to_owned());
        }
        let header = RenderHeader {
            request_id,
            left_id,
            right_id,
            algorithm,
            windows: selection.windows,
            hop_seconds: rendered.config.map(|config| config.hop_seconds),
            render_milliseconds: started.elapsed().as_millis(),
            metrics: rendered.metrics,
        };
        encode_envelope(&header, rendered.wav).map(Response::new)
    })
    .await
    .map_err(|error| format!("render worker failed: {error}"))?
}

#[tauri::command]
fn supersede_render(state: tauri::State<'_, AppState>, request_id: u64) {
    state.latest_request.fetch_max(request_id, Ordering::AcqRel);
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let (manifest, input_dir) = locate_data().map_err(std::io::Error::other)?;
            let renderer =
                OnDemandRenderer::load(&manifest, &input_dir).map_err(std::io::Error::other)?;
            app.manage(AppState {
                renderer: Arc::new(renderer),
                render_lock: Arc::new(Mutex::new(())),
                latest_request: Arc::new(AtomicU64::new(0)),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_bootstrap,
            render_selection,
            supersede_render
        ])
        .run(tauri::generate_context!())
        .expect("error while running the conv9 listener");
}

fn encode_envelope(header: &RenderHeader, wav: Vec<u8>) -> Result<Vec<u8>, String> {
    let header = serde_json::to_vec(header).map_err(|error| error.to_string())?;
    let header_length =
        u32::try_from(header.len()).map_err(|_| "render metadata is too large".to_owned())?;
    let mut envelope = Vec::with_capacity(8 + header.len() + wav.len());
    envelope.extend_from_slice(ENVELOPE_MAGIC);
    envelope.extend_from_slice(&header_length.to_le_bytes());
    envelope.extend_from_slice(&header);
    envelope.extend_from_slice(&wav);
    Ok(envelope)
}

fn locate_data() -> Result<(PathBuf, PathBuf), String> {
    let source_tree = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest = env::var_os("CONV9_MANIFEST")
        .map(PathBuf::from)
        .unwrap_or_else(|| source_tree.join("sources.tsv"));
    let input_dir = env::var_os("CONV9_INPUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| source_tree.join("samples/prepared"));
    let manifest = canonical_file(&manifest, "source manifest")?;
    let input_dir = canonical_directory(&input_dir, "prepared input directory")?;
    Ok((manifest, input_dir))
}

fn canonical_file(path: &Path, label: &str) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("open {label} {}: {error}", path.display()))?;
    if !canonical.is_file() {
        return Err(format!("{} is not a file", canonical.display()));
    }
    Ok(canonical)
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("open {label} {}: {error}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!("{} is not a directory", canonical.display()));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_tree_data_resolves_without_outputs() {
        let (manifest, input_dir) = locate_data().unwrap();
        assert_eq!(
            manifest.file_name().and_then(|name| name.to_str()),
            Some("sources.tsv")
        );
        assert!(input_dir.join("ambient_guitar.wav").is_file());
    }
}
