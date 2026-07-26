use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
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
    render_coordinator: Arc<Mutex<RenderCoordinator>>,
}

#[derive(Default)]
struct RenderCoordinator {
    epoch: u64,
    latest_request: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Bootstrap {
    catalog: Catalog,
    render_epoch: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RenderHeader {
    render_epoch: u64,
    request_id: u64,
    left_id: String,
    right_id: String,
    algorithm: String,
    windows: HashMap<String, f32>,
    parameters: AlgorithmParameters,
    hop_seconds: Option<f32>,
    render_milliseconds: u128,
    timings: conv9::RenderTimings,
    metrics: conv9::AudioMetrics,
}

#[tauri::command]
fn load_bootstrap(state: tauri::State<'_, AppState>) -> Result<Bootstrap, String> {
    let mut coordinator = state
        .render_coordinator
        .lock()
        .map_err(|_| "render coordinator lock was poisoned".to_owned())?;
    coordinator.epoch = coordinator.epoch.saturating_add(1);
    coordinator.latest_request = 0;
    Ok(Bootstrap {
        catalog: state.renderer.catalog(),
        render_epoch: coordinator.epoch,
    })
}

#[tauri::command]
async fn render_selection(
    state: tauri::State<'_, AppState>,
    render_epoch: u64,
    request_id: u64,
    left_id: String,
    right_id: String,
    algorithm: String,
    windows: HashMap<String, f32>,
    parameters: AlgorithmParameters,
) -> Result<Response, String> {
    {
        let mut coordinator = state
            .render_coordinator
            .lock()
            .map_err(|_| "render coordinator lock was poisoned".to_owned())?;
        if coordinator.epoch != render_epoch {
            return Err("render session expired".to_owned());
        }
        coordinator.latest_request = coordinator.latest_request.max(request_id);
    }
    let renderer = Arc::clone(&state.renderer);
    let render_lock = Arc::clone(&state.render_lock);
    let render_coordinator = Arc::clone(&state.render_coordinator);
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = render_lock
            .lock()
            .map_err(|_| "render coordinator lock was poisoned".to_owned())?;
        let cancelled = || {
            render_coordinator
                .lock()
                .map(|coordinator| {
                    coordinator.epoch != render_epoch || coordinator.latest_request != request_id
                })
                .unwrap_or(true)
        };
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
            render_epoch,
            request_id,
            left_id,
            right_id,
            algorithm,
            windows: selection.windows,
            parameters: selection.parameters,
            hop_seconds: rendered.config.map(|config| config.hop_seconds),
            render_milliseconds: started.elapsed().as_millis(),
            timings: rendered.timings,
            metrics: rendered.metrics,
        };
        encode_envelope(&header, rendered.wav).map(Response::new)
    })
    .await
    .map_err(|error| format!("render worker failed: {error}"))?
}

#[tauri::command]
fn supersede_render(state: tauri::State<'_, AppState>, render_epoch: u64, request_id: u64) {
    if let Ok(mut coordinator) = state.render_coordinator.lock()
        && coordinator.epoch == render_epoch
    {
        coordinator.latest_request = coordinator.latest_request.max(request_id);
    }
}

#[tauri::command]
async fn source_preview(
    state: tauri::State<'_, AppState>,
    id: String,
    bins: usize,
) -> Result<conv9::SourcePreview, String> {
    let renderer = Arc::clone(&state.renderer);
    tauri::async_runtime::spawn_blocking(move || {
        renderer
            .source_preview(&id, bins)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("source preview worker failed: {error}"))?
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
                render_coordinator: Arc::new(Mutex::new(RenderCoordinator::default())),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_bootstrap,
            render_selection,
            supersede_render,
            source_preview
        ])
        .run(tauri::generate_context!())
        .expect("error while running the convolution playground");
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

    #[test]
    fn lazy_source_preview_is_bounded_and_finite() {
        let (manifest, input_dir) = locate_data().unwrap();
        let renderer = OnDemandRenderer::load(&manifest, &input_dir).unwrap();
        let preview = renderer.source_preview("ambient_guitar", 128).unwrap();
        assert_eq!(preview.id, "ambient_guitar");
        assert_eq!(preview.peaks.len(), 128);
        assert!(preview.peak.is_finite() && preview.peak > 0.0);
        assert!(preview.rms_dbfs.is_finite());
        assert!((0.0..=1.0).contains(&preview.zero_crossing_rate));
        assert!(
            preview
                .peaks
                .iter()
                .flatten()
                .all(|sample| sample.is_finite())
        );
    }
}
