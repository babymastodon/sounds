mod audio;
mod dsp;
mod latent_convbank;
mod manifest;
mod moving_ir;
mod render;
pub mod synthetic;

pub use audio::AudioMetrics;
pub use dsp::{
    Algorithm, AlgorithmParameters, DEFAULT_A_WINDOW_SECONDS, DEFAULT_B_WINDOW_SECONDS,
    MAX_WINDOW_SECONDS, MIN_WINDOW_SECONDS, WindowConfig,
};
pub use render::{
    AlgorithmCatalogEntry, Catalog, OnDemandRenderer, ParameterCatalogEntry, RenderSelection,
    RenderTimings, RenderedAudio, SourcePreview, WindowCatalogEntry,
};
