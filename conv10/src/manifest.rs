use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

pub const REQUIRED_DOMAINS: [&str; 24] = [
    "cross_01",
    "cross_02",
    "cross_03",
    "cross_04",
    "cross_05",
    "cross_06",
    "cross_07",
    "cross_08",
    "cross_09",
    "cross_10",
    "cross_11",
    "cross_12",
    "cross_13",
    "cross_14",
    "cross_15",
    "cross_16",
    "cross_17",
    "cross_18",
    "cross_19",
    "cross_20",
    "cross_21",
    "cross_22",
    "cross_23",
    "cross_24",
];

#[derive(Clone, Debug, Deserialize)]
pub struct SourceEntry {
    pub id: String,
    pub category: String,
    pub domain: String,
    pub seconds: f64,
    pub trim_start: f64,
    pub provider: String,
    pub creator: String,
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
        if !is_short_duration(entry.seconds) && !is_long_duration(entry.seconds) {
            bail!(
                "{} has duration outside 5..=15 or 25..=35 seconds",
                entry.id
            );
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
    let short_count = entries
        .iter()
        .filter(|entry| is_short_duration(entry.seconds))
        .count();
    if short_count != 24 {
        bail!("expected exactly 24 sources from 5 through 15 seconds; found {short_count}");
    }
    let long_count = entries
        .iter()
        .filter(|entry| is_long_duration(entry.seconds))
        .count();
    if long_count != 24 {
        bail!("expected exactly 24 sources from 25 through 35 seconds; found {long_count}");
    }
    let categories = entries
        .iter()
        .map(|entry| entry.category.as_str())
        .collect::<HashSet<_>>();
    if categories.len() != entries.len() {
        bail!("every source must have a distinct descriptive category");
    }
    for domain in REQUIRED_DOMAINS {
        let matching = entries
            .iter()
            .filter(|entry| entry.domain == domain)
            .collect::<Vec<_>>();
        if matching.len() != 2 {
            bail!(
                "expected exactly 2 {domain} sources; found {}",
                matching.len()
            );
        }
        let domain_short = matching
            .iter()
            .filter(|entry| is_short_duration(entry.seconds))
            .count();
        let domain_long = matching
            .iter()
            .filter(|entry| is_long_duration(entry.seconds))
            .count();
        if domain_short != 1 || domain_long != 1 {
            bail!("{domain} must contain exactly one short and one long source");
        }
    }
    for entry in &entries {
        if entry.domain.trim().is_empty()
            || entry.provider.trim().is_empty()
            || entry.creator.trim().is_empty()
        {
            bail!(
                "{} is missing domain, provider, or creator provenance",
                entry.id
            );
        }
        for (label, url) in [
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

pub fn is_short_duration(seconds: f64) -> bool {
    (5.0..=15.0).contains(&seconds)
}

pub fn is_long_duration(seconds: f64) -> bool {
    (25.0..=35.0).contains(&seconds)
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
                .filter(|entry| is_short_duration(entry.seconds))
                .count(),
            24
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| is_long_duration(entry.seconds))
                .count(),
            24
        );
        for domain in REQUIRED_DOMAINS {
            let matching = entries
                .iter()
                .filter(|entry| entry.domain == domain)
                .collect::<Vec<_>>();
            assert_eq!(matching.len(), 2);
            assert_eq!(
                matching
                    .iter()
                    .filter(|entry| is_short_duration(entry.seconds))
                    .count(),
                1
            );
            assert_eq!(
                matching
                    .iter()
                    .filter(|entry| is_long_duration(entry.seconds))
                    .count(),
                1
            );
        }
    }
}
