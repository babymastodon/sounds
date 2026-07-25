use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::Manager;

struct AppState {
    output_dir: PathBuf,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Bootstrap {
    output_dir: String,
    catalog: serde_json::Value,
}

#[tauri::command]
fn load_bootstrap(state: tauri::State<'_, AppState>) -> Result<Bootstrap, String> {
    let catalog_path = state.output_dir.join("catalog.json");
    let bytes = fs::read(&catalog_path)
        .map_err(|error| format!("read {}: {error}", catalog_path.display()))?;
    let catalog = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse {}: {error}", catalog_path.display()))?;
    Ok(Bootstrap {
        output_dir: state.output_dir.to_string_lossy().into_owned(),
        catalog,
    })
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let output_dir = locate_output_dir().map_err(std::io::Error::other)?;
            app.asset_protocol_scope()
                .allow_directory(&output_dir, true)
                .map_err(std::io::Error::other)?;
            app.manage(AppState { output_dir });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![load_bootstrap])
        .run(tauri::generate_context!())
        .expect("error while running the conv9 listener");
}

fn locate_output_dir() -> Result<PathBuf, String> {
    if let Some(configured) = env::var_os("CONV9_OUTPUT_DIR") {
        return validate_output_dir(PathBuf::from(configured));
    }
    let source_tree = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("outputs");
    if source_tree.join("catalog.json").is_file() {
        return validate_output_dir(source_tree);
    }
    let current = env::current_dir().map_err(|error| error.to_string())?;
    for ancestor in current.ancestors() {
        let candidate = ancestor.join("outputs");
        if candidate.join("catalog.json").is_file() {
            return validate_output_dir(candidate);
        }
        let candidate = ancestor.join("conv9").join("outputs");
        if candidate.join("catalog.json").is_file() {
            return validate_output_dir(candidate);
        }
    }
    Err(
        "could not find conv9/outputs/catalog.json; set CONV9_OUTPUT_DIR to the output directory"
            .to_owned(),
    )
}

fn validate_output_dir(path: PathBuf) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("open output directory {}: {error}", path.display()))?;
    if !canonical.join("catalog.json").is_file() {
        return Err(format!(
            "{} does not contain catalog.json",
            canonical.display()
        ));
    }
    Ok(canonical)
}
