use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

pub const SOURCE_COUNT: usize = 96;
pub const SOURCE_SECONDS: f64 = 61.0;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SourceEntry {
    pub id: String,
    pub category: String,
    pub kind: String,
    pub seconds: f64,
    pub trim_start: f64,
    pub provider: String,
    pub creator: String,
    pub license: String,
    pub license_url: String,
    pub source_page: String,
    pub download_url: String,
    pub cache_source: String,
}

pub fn load_manifest(path: &Path) -> Result<Vec<SourceEntry>> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(b'\t')
        .from_path(path)
        .with_context(|| format!("open manifest {}", path.display()))?;
    let mut entries = Vec::new();
    for row in reader.deserialize() {
        let entry: SourceEntry = row.context("parse source manifest row")?;
        validate_entry(&entry)?;
        if entries
            .iter()
            .any(|prior: &SourceEntry| prior.id == entry.id)
        {
            bail!("duplicate source id {}", entry.id);
        }
        entries.push(entry);
    }
    if entries.len() != SOURCE_COUNT {
        bail!("expected {SOURCE_COUNT} sources, found {}", entries.len());
    }
    let kinds = entries
        .iter()
        .map(|entry| entry.kind.as_str())
        .collect::<HashSet<_>>();
    if kinds.len() != entries.len() {
        bail!("every source must represent a distinct sound kind");
    }
    let urls = entries
        .iter()
        .map(|entry| entry.download_url.as_str())
        .collect::<HashSet<_>>();
    if urls.len() != entries.len() {
        bail!("every source must have a distinct download URL");
    }
    Ok(entries)
}

fn validate_entry(entry: &SourceEntry) -> Result<()> {
    if entry.id.is_empty()
        || !entry
            .id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        bail!("invalid source id {:?}", entry.id);
    }
    if (entry.seconds - SOURCE_SECONDS).abs() > 1.0e-6 {
        bail!("{} must be exactly {SOURCE_SECONDS} seconds", entry.id);
    }
    if entry.trim_start < 0.0 || !entry.trim_start.is_finite() {
        bail!("{} has an invalid trim offset", entry.id);
    }
    for (name, value) in [
        ("category", entry.category.as_str()),
        ("kind", entry.kind.as_str()),
        ("provider", entry.provider.as_str()),
        ("creator", entry.creator.as_str()),
    ] {
        if value.trim().is_empty() {
            bail!("{} is missing {name}", entry.id);
        }
    }
    if entry.license != "CC0 1.0" {
        bail!("{} must use CC0 1.0, found {}", entry.id, entry.license);
    }
    let expected_license_url = "https://creativecommons.org/publicdomain/zero/1.0/";
    if entry.license_url != expected_license_url {
        bail!("{} has a mismatched license URL", entry.id);
    }
    for (name, value) in [
        ("license", entry.license_url.as_str()),
        ("source", entry.source_page.as_str()),
        ("download", entry.download_url.as_str()),
    ] {
        if !value.starts_with("https://") {
            bail!("{} has a non-HTTPS {name} URL", entry.id);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_manifest_is_valid() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("sources.tsv");
        let entries = load_manifest(&path).unwrap();
        assert_eq!(entries.len(), SOURCE_COUNT);
        assert!(entries.iter().all(|source| source.seconds > 60.0));
        assert!(entries.iter().all(|source| {
            source.license == "CC0 1.0"
                && source.license_url == "https://creativecommons.org/publicdomain/zero/1.0/"
        }));
        assert_eq!(
            entries
                .iter()
                .map(|source| source.kind.as_str())
                .collect::<HashSet<_>>()
                .len(),
            SOURCE_COUNT
        );
    }
}
