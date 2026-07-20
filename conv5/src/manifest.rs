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
        if !(10.0..=60.0).contains(&entry.seconds) {
            bail!("{} has duration outside 10..=60 seconds", entry.id);
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
    let short_count = entries.iter().filter(|entry| entry.seconds <= 35.0).count();
    if short_count != 24 {
        bail!("expected exactly 24 sources from 10 through 35 seconds; found {short_count}");
    }
    let long_count = entries.iter().filter(|entry| entry.seconds > 35.0).count();
    if long_count != 24 {
        bail!("expected exactly 24 sources over 35 through 60 seconds; found {long_count}");
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
    for domain in [
        "busy_city",
        "industrial",
        "rain",
        "sports",
        "long_instrument",
        "speeches",
        "train_ambient",
        "walking",
    ] {
        let count = entries
            .iter()
            .filter(|entry| entry.domain == domain)
            .count();
        if count != 6 {
            bail!("expected exactly 6 {domain} sources; found {count}");
        }
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
        let expected_license_url = match entry.license.as_str() {
            "CC0 1.0" => "https://creativecommons.org/publicdomain/zero/1.0/",
            "CC BY 3.0" => "https://creativecommons.org/licenses/by/3.0/",
            "CC BY 4.0" => "https://creativecommons.org/licenses/by/4.0/",
            "CC BY-SA 3.0" => "https://creativecommons.org/licenses/by-sa/3.0/",
            "CC BY-SA 4.0" => "https://creativecommons.org/licenses/by-sa/4.0/",
            _ => bail!("{} does not declare an approved open license", entry.id),
        };
        if entry.license_url != expected_license_url {
            bail!(
                "{} has a license URL that does not match {}",
                entry.id,
                entry.license
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
            entries.iter().filter(|entry| entry.seconds <= 35.0).count(),
            24
        );
        assert_eq!(
            entries.iter().filter(|entry| entry.seconds > 35.0).count(),
            24
        );
        for domain in [
            "busy_city",
            "industrial",
            "rain",
            "sports",
            "long_instrument",
            "speeches",
            "train_ambient",
            "walking",
        ] {
            assert_eq!(
                entries
                    .iter()
                    .filter(|entry| entry.domain == domain)
                    .count(),
                6
            );
        }
    }
}
