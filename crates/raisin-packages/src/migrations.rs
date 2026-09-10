// SPDX-License-Identifier: BSL-1.1

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Seek};
use zip::ZipArchive;

use crate::error::{PackageError, PackageResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageMigration {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub operations: Vec<serde_yaml::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationSummary {
    pub id: String,
    pub path: String,
    pub title: Option<String>,
    pub operations: usize,
}

pub fn read_migration_summaries<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> PackageResult<Vec<MigrationSummary>> {
    let mut summaries = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let path = entry.name().to_string();
        if !path.starts_with("migrations/") || !path.ends_with(".yaml") || entry.is_dir() {
            continue;
        }
        let mut content = String::new();
        entry
            .read_to_string(&mut content)
            .map_err(PackageError::IoError)?;
        let migration: PackageMigration =
            serde_yaml::from_str(&content).map_err(PackageError::YamlError)?;
        if migration.id.trim().is_empty() {
            return Err(PackageError::InvalidManifest(format!(
                "migration '{}' must declare id",
                path
            )));
        }
        summaries.push(MigrationSummary {
            id: migration.id,
            path,
            title: migration.title,
            operations: migration.operations.len(),
        });
    }
    summaries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(summaries)
}

pub fn summaries_to_yaml_value(
    summaries: &[MigrationSummary],
) -> Vec<HashMap<String, serde_yaml::Value>> {
    summaries
        .iter()
        .map(|summary| {
            let mut item = HashMap::new();
            item.insert(
                "id".to_string(),
                serde_yaml::Value::String(summary.id.clone()),
            );
            item.insert(
                "path".to_string(),
                serde_yaml::Value::String(summary.path.clone()),
            );
            if let Some(title) = &summary.title {
                item.insert(
                    "title".to_string(),
                    serde_yaml::Value::String(title.clone()),
                );
            }
            item.insert(
                "operations".to_string(),
                serde_yaml::Value::Number(summary.operations.into()),
            );
            item
        })
        .collect()
}
