//! Spillable CTE Storage with MessagePack Serialization
//!
//! This module provides automatic memory-to-disk spillage for Common Table Expression (CTE)
//! materialization. When a CTE's result set exceeds configured memory limits, rows are
//! transparently spilled to temporary disk files using MessagePack serialization.
//!
//! # Architecture
//!
//! - **In-Memory Storage**: Small CTEs (< 10MB default) kept entirely in memory
//! - **Disk Spillage**: Large CTEs automatically spill to temp files with first 100 rows cached
//! - **MessagePack Encoding**: Efficient binary serialization with ~2x compression vs JSON
//! - **Transparent Iteration**: Iterator API works seamlessly for both memory and disk storage
//!
//! # Module Structure
//!
//! - `size_estimation` - Memory footprint estimation for spillage decisions

mod size_estimation;

use super::executor::{execute_plan, ExecutionContext, ExecutionError, Row};
use super::operators::PhysicalPlan;
use futures::StreamExt;
use raisin_storage::Storage;
use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use tracing::{debug, info, warn};

pub use size_estimation::{estimate_property_value_size, estimate_row_size};

/// Configuration for CTE materialization and spillage behavior
///
/// Controls when CTEs spill from memory to disk and where temporary files are stored.
#[derive(Debug, Clone)]
pub struct CTEConfig {
    /// Memory limit per CTE before spilling to disk (in bytes)
    ///
    /// Default: 10MB (10_485_760 bytes)
    pub per_cte_memory_limit: usize,

    /// Directory for temporary spill files
    ///
    /// Default: std::env::temp_dir()
    pub temp_dir: PathBuf,
}

impl Default for CTEConfig {
    fn default() -> Self {
        let per_cte_memory_limit = std::env::var("RAISIN_CTE_MEMORY_LIMIT")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(10 * 1024 * 1024);

        Self {
            per_cte_memory_limit,
            temp_dir: std::env::temp_dir(),
        }
    }
}

impl CTEConfig {
    /// Create a new configuration with specified memory limit
    pub fn new(memory_limit: usize) -> Self {
        Self {
            per_cte_memory_limit: memory_limit,
            temp_dir: std::env::temp_dir(),
        }
    }

    /// Set the temporary directory for spill files
    pub fn with_temp_dir(mut self, temp_dir: PathBuf) -> Self {
        self.temp_dir = temp_dir;
        self
    }
}

/// Materialized CTE result set with automatic memory-to-disk spillage
///
/// Transparently handles both in-memory and disk-spilled CTE results.
pub enum MaterializedCTE {
    /// All rows stored in memory (below configured memory limit)
    InMemory {
        /// All rows materialized from the CTE
        rows: Vec<Row>,
        /// Total estimated memory usage in bytes
        size_bytes: usize,
    },

    /// Rows spilled to disk with memory cache for first 100 rows
    OnDisk {
        /// Path to the temporary spill file
        file_path: PathBuf,
        /// Total number of rows in the CTE
        row_count: usize,
        /// Total size of the spill file in bytes
        size_bytes: usize,
        /// First 100 rows cached in memory for small scans
        memory_cache: Vec<Row>,
    },
}

impl MaterializedCTE {
    /// Materialize a CTE by executing its physical plan
    ///
    /// Automatically spills to disk if memory limits are exceeded.
    pub async fn materialize<
        S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    >(
        plan: &PhysicalPlan,
        ctx: &ExecutionContext<S>,
        config: &CTEConfig,
    ) -> Result<Self, ExecutionError> {
        let mut stream = execute_plan(plan, ctx).await?;
        let mut rows = Vec::new();
        let mut total_size: usize = 0;

        while let Some(result) = stream.next().await {
            let row = result?;
            let row_size = estimate_row_size(&row);
            total_size += row_size;

            rows.push(row);

            if total_size > config.per_cte_memory_limit && rows.len() > 100 {
                info!(
                    "CTE materialization exceeding memory limit ({} bytes > {} bytes), spilling to disk",
                    total_size, config.per_cte_memory_limit
                );

                let (file_path, file_size) = Self::spill_to_disk(&rows, &config.temp_dir)?;
                let additional_rows = Self::append_to_disk(&file_path, &mut stream).await?;
                let total_rows = rows.len() + additional_rows;

                rows.truncate(100);

                debug!(
                    "CTE spilled to disk: {} rows, {} bytes, cache: {} rows",
                    total_rows,
                    file_size,
                    rows.len()
                );

                return Ok(MaterializedCTE::OnDisk {
                    file_path,
                    row_count: total_rows,
                    size_bytes: file_size,
                    memory_cache: rows,
                });
            }
        }

        debug!(
            "CTE materialized in memory: {} rows, {} bytes",
            rows.len(),
            total_size
        );

        Ok(MaterializedCTE::InMemory {
            rows,
            size_bytes: total_size,
        })
    }

    /// Spill rows to a temporary disk file using MessagePack serialization
    fn spill_to_disk(rows: &[Row], temp_dir: &Path) -> Result<(PathBuf, usize), ExecutionError> {
        let temp_file = NamedTempFile::new_in(temp_dir).map_err(|e| {
            ExecutionError::Backend(format!(
                "Failed to create temp file for CTE spillage: {}",
                e
            ))
        })?;

        let file_path = temp_file.path().to_path_buf();
        let file = temp_file
            .persist(&file_path)
            .map_err(|e| ExecutionError::Backend(format!("Failed to persist temp file: {}", e)))?;

        let mut writer = BufWriter::new(file);

        for row in rows {
            Self::write_row_to_file(&mut writer, row)?;
        }

        writer
            .flush()
            .map_err(|e| ExecutionError::Backend(format!("Failed to flush spill file: {}", e)))?;

        let metadata = std::fs::metadata(&file_path)
            .map_err(|e| ExecutionError::Backend(format!("Failed to get file metadata: {}", e)))?;

        Ok((file_path, metadata.len() as usize))
    }

    /// Append remaining rows from stream directly to disk file
    async fn append_to_disk(
        file_path: &Path,
        stream: &mut futures::stream::BoxStream<'_, Result<Row, ExecutionError>>,
    ) -> Result<usize, ExecutionError> {
        let file = std::fs::OpenOptions::new()
            .append(true)
            .open(file_path)
            .map_err(|e| {
                ExecutionError::Backend(format!("Failed to open spill file for append: {}", e))
            })?;

        let mut writer = BufWriter::new(file);
        let mut count = 0;

        while let Some(result) = stream.next().await {
            let row = result?;
            Self::write_row_to_file(&mut writer, &row)?;
            count += 1;
        }

        writer
            .flush()
            .map_err(|e| ExecutionError::Backend(format!("Failed to flush spill file: {}", e)))?;

        Ok(count)
    }

    /// Write a single row to a file using MessagePack serialization
    fn write_row_to_file<W: Write>(writer: &mut W, row: &Row) -> Result<(), ExecutionError> {
        rmp_serde::encode::write(writer, &row.columns).map_err(|e| {
            ExecutionError::Backend(format!("Failed to serialize row to MessagePack: {}", e))
        })
    }

    /// Read a single row from a MessagePack file
    fn read_row_from_file(reader: &mut BufReader<File>) -> Result<Option<Row>, ExecutionError> {
        match rmp_serde::decode::from_read(reader) {
            Ok(columns) => Ok(Some(Row::from_map(columns))),
            Err(rmp_serde::decode::Error::InvalidMarkerRead(ref io_err))
                if io_err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                Ok(None)
            }
            Err(e) => Err(ExecutionError::Backend(format!(
                "Failed to deserialize row from MessagePack: {}",
                e
            ))),
        }
    }

    /// Create an iterator over the materialized CTE results
    pub fn iter(&self) -> Result<CTEIterator, ExecutionError> {
        match self {
            MaterializedCTE::InMemory { rows, .. } => Ok(CTEIterator::Memory {
                rows: rows.clone(),
                position: 0,
            }),
            MaterializedCTE::OnDisk { file_path, .. } => {
                let file = File::open(file_path).map_err(|e| {
                    ExecutionError::Backend(format!("Failed to open CTE spill file: {}", e))
                })?;
                let reader = BufReader::new(file);
                Ok(CTEIterator::Disk { reader })
            }
        }
    }

    /// Get the number of rows in the materialized CTE
    pub fn row_count(&self) -> usize {
        match self {
            MaterializedCTE::InMemory { rows, .. } => rows.len(),
            MaterializedCTE::OnDisk { row_count, .. } => *row_count,
        }
    }

    /// Get the total size in bytes
    pub fn size_bytes(&self) -> usize {
        match self {
            MaterializedCTE::InMemory { size_bytes, .. } => *size_bytes,
            MaterializedCTE::OnDisk { size_bytes, .. } => *size_bytes,
        }
    }

    /// Check if the CTE is stored in memory
    pub fn is_in_memory(&self) -> bool {
        matches!(self, MaterializedCTE::InMemory { .. })
    }

    /// Check if the CTE is spilled to disk
    pub fn is_on_disk(&self) -> bool {
        matches!(self, MaterializedCTE::OnDisk { .. })
    }
}

impl Drop for MaterializedCTE {
    fn drop(&mut self) {
        if let MaterializedCTE::OnDisk { file_path, .. } = self {
            let path = file_path.clone();
            if let Err(e) = std::fs::remove_file(&path) {
                warn!("Failed to clean up CTE spill file {:?}: {}", path, e);
            } else {
                debug!("Cleaned up CTE spill file: {:?}", path);
            }
        }
    }
}

/// Iterator over materialized CTE results
///
/// Transparently handles both in-memory and disk-spilled storage.
pub enum CTEIterator {
    /// Iterator over in-memory rows
    Memory { rows: Vec<Row>, position: usize },
    /// Iterator over disk-spilled rows
    Disk { reader: BufReader<File> },
}

impl Iterator for CTEIterator {
    type Item = Result<Row, ExecutionError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            CTEIterator::Memory { rows, position } => {
                if *position < rows.len() {
                    let row = rows[*position].clone();
                    *position += 1;
                    Some(Ok(row))
                } else {
                    None
                }
            }
            CTEIterator::Disk { reader } => match MaterializedCTE::read_row_from_file(reader) {
                Ok(Some(row)) => Some(Ok(row)),
                Ok(None) => None,
                Err(e) => Some(Err(e)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::properties::PropertyValue;

    #[test]
    fn test_cte_config_default() {
        let config = CTEConfig::default();
        assert!(config.per_cte_memory_limit > 0);
        assert!(config.temp_dir.exists());
    }

    #[test]
    fn test_cte_config_custom() {
        let config = CTEConfig::new(5 * 1024 * 1024);
        assert_eq!(config.per_cte_memory_limit, 5 * 1024 * 1024);
    }

    #[test]
    fn test_cte_config_with_temp_dir() {
        let temp_dir = PathBuf::from("/tmp/custom");
        let config = CTEConfig::default().with_temp_dir(temp_dir.clone());
        assert_eq!(config.temp_dir, temp_dir);
    }

    #[test]
    fn test_write_and_read_row() {
        let mut row = Row::new();
        row.insert("id".to_string(), PropertyValue::String("test".to_string()));
        row.insert("value".to_string(), PropertyValue::Integer(42));

        let temp_dir = std::env::temp_dir();
        let temp_file = NamedTempFile::new_in(&temp_dir).unwrap();
        let file_path = temp_file.path().to_path_buf();

        {
            let file = temp_file.persist(&file_path).unwrap();
            let mut writer = BufWriter::new(file);
            MaterializedCTE::write_row_to_file(&mut writer, &row).unwrap();
            writer.flush().unwrap();
        }

        let file = File::open(&file_path).unwrap();
        let mut reader = BufReader::new(file);
        let read_row = MaterializedCTE::read_row_from_file(&mut reader)
            .unwrap()
            .unwrap();

        assert_eq!(read_row.columns.len(), 2);
        assert_eq!(
            read_row.get("id"),
            Some(&PropertyValue::String("test".to_string()))
        );
        assert_eq!(read_row.get("value"), Some(&PropertyValue::Integer(42)));

        std::fs::remove_file(file_path).ok();
    }

    #[test]
    fn test_multiple_rows_write_and_read() {
        let rows = vec![
            {
                let mut row = Row::new();
                row.insert("id".to_string(), PropertyValue::Integer(1));
                row.insert(
                    "name".to_string(),
                    PropertyValue::String("Alice".to_string()),
                );
                row
            },
            {
                let mut row = Row::new();
                row.insert("id".to_string(), PropertyValue::Integer(2));
                row.insert("name".to_string(), PropertyValue::String("Bob".to_string()));
                row
            },
            {
                let mut row = Row::new();
                row.insert("id".to_string(), PropertyValue::Integer(3));
                row.insert(
                    "name".to_string(),
                    PropertyValue::String("Charlie".to_string()),
                );
                row
            },
        ];

        let temp_dir = std::env::temp_dir();
        let temp_file = NamedTempFile::new_in(&temp_dir).unwrap();
        let file_path = temp_file.path().to_path_buf();

        {
            let file = temp_file.persist(&file_path).unwrap();
            let mut writer = BufWriter::new(file);
            for row in &rows {
                MaterializedCTE::write_row_to_file(&mut writer, row).unwrap();
            }
            writer.flush().unwrap();
        }

        let file = File::open(&file_path).unwrap();
        let mut reader = BufReader::new(file);
        let mut read_rows = Vec::new();

        while let Some(row) = MaterializedCTE::read_row_from_file(&mut reader).unwrap() {
            read_rows.push(row);
        }

        assert_eq!(read_rows.len(), 3);
        assert_eq!(read_rows[0].get("id"), Some(&PropertyValue::Integer(1)));
        assert_eq!(
            read_rows[0].get("name"),
            Some(&PropertyValue::String("Alice".to_string()))
        );
        assert_eq!(read_rows[2].get("id"), Some(&PropertyValue::Integer(3)));

        std::fs::remove_file(file_path).ok();
    }
}
