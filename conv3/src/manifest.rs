use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct SourceEntry {
    pub id: String,
    pub category: String,
    pub domain: String,
    pub seconds: f64,
    pub trim_start: f64,
    pub provider: String,
    pub creator: String,
    pub license: String,
    pub license_url: String,
    pub source_page: String,
    pub download_url: String,
}

pub fn load_manifest(path: &Path) -> Result<Vec<SourceEntry>> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(b'\t')
        .from_path(path)
        .with_context(|| format!("open manifest {}", path.display()))?;
    let mut entries = Vec::new();

    for row in reader.deserialize() {
        let entry: SourceEntry = row.context("parse source manifest row")?;
        if entry.id.is_empty()
            || !entry
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            bail!(
                "source id must contain only lowercase ASCII, digits, and _: {:?}",
                entry.id
            );
        }
        if !(1.0..=60.0).contains(&entry.seconds) {
            bail!("{} has duration outside 1..=60 seconds", entry.id);
        }
        if entry.trim_start < 0.0 {
            bail!("{} has a negative trim offset", entry.id);
        }
        if entries
            .iter()
            .any(|existing: &SourceEntry| existing.id == entry.id)
        {
            bail!("duplicate source id {}", entry.id);
        }
        entries.push(entry);
    }

    if entries.len() != 48 {
        bail!("expected exactly 48 sources; found {}", entries.len());
    }
    let long_count = entries
        .iter()
        .filter(|entry| entry.seconds > 30.0 && entry.seconds <= 60.0)
        .count();
    if long_count != 24 {
        bail!("expected exactly 24 sources over 30 through 60 seconds; found {long_count}");
    }
    let providers = entries
        .iter()
        .map(|entry| entry.provider.as_str())
        .collect::<HashSet<_>>();
    if providers.len() < 3 {
        bail!("expected at least three independent source providers");
    }
    let categories = entries
        .iter()
        .map(|entry| entry.category.as_str())
        .collect::<HashSet<_>>();
    if categories.len() < 40 {
        bail!("expected at least forty distinct ambient categories");
    }
    let industrial_count = entries
        .iter()
        .filter(|entry| entry.domain == "industrial")
        .count();
    if industrial_count * 2 < entries.len() {
        bail!(
            "at least half of ambient sources must be industrial; found {industrial_count}/{}",
            entries.len()
        );
    }
    for entry in &entries {
        if entry.domain.trim().is_empty()
            || entry.creator.trim().is_empty()
            || entry.license.trim().is_empty()
        {
            bail!(
                "{} is missing domain, creator, or license provenance",
                entry.id
            );
        }
        for (label, url) in [
            ("license", entry.license_url.as_str()),
            ("source", entry.source_page.as_str()),
            ("download", entry.download_url.as_str()),
        ] {
            if !url.starts_with("https://") {
                bail!("{} has a non-HTTPS {label} URL", entry.id);
            }
        }
    }
    let download_urls = entries
        .iter()
        .map(|entry| entry.download_url.as_str())
        .collect::<HashSet<_>>();
    if download_urls.len() != entries.len() {
        bail!("every source must have a distinct download URL");
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_in_manifest_has_the_required_shape() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("sources.tsv");
        let entries = load_manifest(&path).unwrap();

        assert_eq!(entries.len(), 48);
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.seconds > 30.0 && entry.seconds <= 60.0)
                .count(),
            24
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.domain == "industrial")
                .count(),
            25
        );
    }
}
