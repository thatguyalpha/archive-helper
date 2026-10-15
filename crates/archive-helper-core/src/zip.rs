use crate::atomic::AtomicFileWriter;
use crate::error::{ArchiveError, ArchiveWarning};
use crate::fs::{
    is_same_or_child_path, open_resilient_read, safe_delete_dir_all, safe_delete_file,
    strip_extended_prefix, validate_archive_name,
};
use crate::progress::{CancellationToken, OperationStage, ProgressReporter, ProgressUpdate};
use crate::scanner::{scan_sources, ArchiveExtension, PreScanResult};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

/// Configuration options for zip archive creation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ZipOptions {
    /// Whether to delete original source files/folders after successful archive creation.
    pub delete_original: bool,
    /// Automatically predict archive extension (.osz for beatmaps, .osk for skins, .zip otherwise).
    pub predict_ext: bool,
    /// Preserve the latest modification timestamp from source entries onto the archive and its entries.
    pub keep_mtime: bool,
    /// Custom destination directory (defaults to the common ancestor directory of sources).
    pub destination_dir: Option<PathBuf>,
    /// Stream read/write buffer capacity in bytes (default: 1 MiB).
    pub buffer_size: usize,
}

impl Default for ZipOptions {
    fn default() -> Self {
        Self {
            delete_original: false,
            predict_ext: true,
            keep_mtime: true,
            destination_dir: None,
            buffer_size: 1024 * 1024, // 1 MiB
        }
    }
}

/// Result summary of an archive creation operation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ArchiveResult {
    /// Absolute path to the created archive
    pub output_path: PathBuf,
    /// Total bytes written to the archive
    pub bytes_written: u64,
    /// Total regular files successfully written
    pub files_archived: usize,
    /// Total regular files discovered during pre-scan
    pub total_files: usize,
    /// Wall-clock time taken for the entire operation
    pub elapsed: Duration,
    /// Non-fatal warnings encountered (e.g. skipped locked files, failed deletions)
    pub warnings: Vec<ArchiveWarning>,
}

/// Converts a `SystemTime` to a `zip::DateTime` clamped to the valid MS-DOS timestamp range (1980..2107).
pub fn system_time_to_zip_datetime(st: SystemTime) -> Option<DateTime> {
    let offset_dt = time::OffsetDateTime::from(st);
    let mut year = offset_dt.year();
    if year < 1980 {
        year = 1980;
    } else if year > 2107 {
        year = 2107;
    }

    let primitive = time::PrimitiveDateTime::new(
        time::Date::from_calendar_date(year, offset_dt.month(), offset_dt.day()).ok()?,
        offset_dt.time(),
    );
    DateTime::try_from(primitive).ok()
}

/// Creates a zero-compression ("Store") zip, osz, or osk archive from the provided sources.
///
/// Features:
/// - Atomic file writing: writes to a hidden `.tmp` file and atomically renames upon completion.
/// - RAII cleanup: any panic, cancellation, or error ensures no 0-byte or corrupted file remains on disk.
/// - Resilient file reading: opened with `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`.
///   Locked or unreadable files are skipped with non-fatal warnings rather than aborting.
/// - Zero-compression sequential streaming: uses standard ZIP Data Descriptors without backward seeks,
///   saturating SSD/NVMe write throughput.
pub fn create_archive(
    sources: &[PathBuf],
    custom_name: Option<&str>,
    options: &ZipOptions,
    cancellation: Option<&CancellationToken>,
    progress_tx: Option<crossbeam_channel::Sender<ProgressUpdate>>,
) -> Result<ArchiveResult, ArchiveError> {
    let start_time = Instant::now();
    let mut warnings = Vec::new();

    if let Some(token) = cancellation {
        token.check_cancelled()?;
    }

    // 1. Parallel Pre-scan
    let scan_result = scan_sources(sources, options.predict_ext)?;
    if scan_result.entries.is_empty() {
        return Err(ArchiveError::EmptySource);
    }

    // 2. Resolve Archive Name
    let folder_name = match custom_name {
        Some(name) => {
            validate_archive_name(name)?;
            name.to_string()
        }
        None => derive_archive_name(sources, &scan_result)?,
    };

    // 3. Determine Archive Extension
    let extension = if options.predict_ext {
        scan_result.predicted_extension
    } else {
        ArchiveExtension::Zip
    };

    let full_archive_filename = format!("{}{}", folder_name, extension.as_str());

    // 4. Resolve Output Directory
    let output_dir = match &options.destination_dir {
        Some(dir) => dir.clone(),
        None => scan_result
            .selection_root
            .clone()
            .or_else(|| sources[0].parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from(".")),
    };

    // Ensure output directory is not inside any source folder (circular reference)
    for src in sources {
        if src.is_dir() && is_same_or_child_path(&output_dir, src) {
            return Err(ArchiveError::CircularOutputTarget {
                source_path: src.clone(),
                output_path: output_dir.clone(),
            });
        }
    }

    std::fs::create_dir_all(&output_dir)?;
    let target_path = output_dir.join(&full_archive_filename);

    // Ensure target path does not overwrite an input source file
    for src in sources {
        if src.is_file() && src == &target_path {
            return Err(ArchiveError::TargetOverwritesSource(target_path));
        }
    }

    let operation_id = format!("zip-{}", fastrand::u64(..));
    let mut reporter = ProgressReporter::new(
        operation_id,
        folder_name.clone(),
        scan_result.total_bytes,
        scan_result.total_files,
        progress_tx,
    );

    reporter.send_immediate(OperationStage::Archiving, 0, 0, None);

    // 5. Initialize Atomic File Writer
    let atomic_writer = AtomicFileWriter::with_buffer_size(&target_path, options.buffer_size)?;

    // 6. Initialize Sequential Streaming Zip Writer
    let mut zip_writer = ZipWriter::new_stream(atomic_writer);

    let mut buffer = vec![0u8; options.buffer_size];
    let mut bytes_written = 0u64;
    let mut files_archived = 0usize;

    // 7. Stream Entries Sequentially
    for entry in &scan_result.entries {
        if let Some(token) = cancellation {
            token.check_cancelled()?;
        }

        let mut file_opts = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored);

        if options.keep_mtime {
            if let Some(mod_time) = entry.modified {
                if let Some(zip_dt) = system_time_to_zip_datetime(mod_time) {
                    file_opts = file_opts.last_modified_time(zip_dt);
                }
            }
        }

        if entry.is_dir {
            // Write directory entry
            if let Err(e) = zip_writer.add_directory(&entry.archive_path, file_opts) {
                tracing::warn!("Failed to add directory entry '{}': {}", entry.archive_path, e);
            }
            continue;
        }

        // Open regular file with permissive sharing mode
        let mut file = match open_resilient_read(&entry.disk_path) {
            Ok(f) => f,
            Err(e) => {
                let warn = ArchiveWarning::FileReadSkipped {
                    path: entry.disk_path.clone(),
                    reason: e.to_string(),
                };
                tracing::warn!("{}", warn);
                warnings.push(warn);
                continue;
            }
        };

        // Start file entry in zip archive
        if let Err(e) = zip_writer.start_file(&entry.archive_path, file_opts) {
            let warn = ArchiveWarning::FileReadSkipped {
                path: entry.disk_path.clone(),
                reason: format!("ZipWriter error: {}", e),
            };
            tracing::warn!("{}", warn);
            warnings.push(warn);
            continue;
        }

        let entry_filename = entry
            .disk_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| entry.archive_path.clone());

        // Stream file contents into archive
        let mut file_read_failed = false;
        loop {
            if let Some(token) = cancellation {
                token.check_cancelled()?;
            }

            match file.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if let Err(e) = zip_writer.write_all(&buffer[..n]) {
                        tracing::error!("Write failed to archive for entry '{}': {}", entry.archive_path, e);
                        return Err(ArchiveError::Io(e));
                    }
                    bytes_written += n as u64;
                    reporter.send_throttled(
                        OperationStage::Archiving,
                        bytes_written,
                        files_archived,
                        Some(entry_filename.clone()),
                    );
                }
                Err(e) => {
                    let warn = ArchiveWarning::FileReadSkipped {
                        path: entry.disk_path.clone(),
                        reason: format!("I/O read error midway: {}", e),
                    };
                    tracing::warn!("{}", warn);
                    warnings.push(warn);
                    file_read_failed = true;
                    break;
                }
            }
        }

        if !file_read_failed {
            files_archived += 1;
        }
    }

    // 8. Finalize Central Directory
    reporter.send_immediate(OperationStage::Finalizing, bytes_written, files_archived, None);

    let stream_writer = zip_writer.finish()?;
    let atomic_writer = stream_writer.into_inner();

    // 9. Atomic Commit (rename .tmp to final target)
    let final_path = atomic_writer.commit()?;

    // 10. Set Archive File Last Modified Timestamp
    if options.keep_mtime {
        if let Some(max_time) = scan_result.max_modified {
            let file_time = filetime::FileTime::from_system_time(max_time);
            if let Err(e) = filetime::set_file_mtime(&final_path, file_time) {
                let warn = ArchiveWarning::MetadataFailed {
                    path: final_path.clone(),
                    reason: e.to_string(),
                };
                warnings.push(warn);
            }
        }
    }

    // 11. Delete Original Files/Folders (if requested)
    if options.delete_original {
        reporter.send_immediate(OperationStage::CleaningUp, bytes_written, files_archived, None);
        for src in sources {
            let res = if src.is_dir() {
                safe_delete_dir_all(src, 5, Duration::from_millis(150))
            } else {
                safe_delete_file(src, 5, Duration::from_millis(150))
            };

            if let Err(e) = res {
                let warn = ArchiveWarning::DeletionFailed {
                    path: src.clone(),
                    reason: e.to_string(),
                };
                tracing::warn!("{}", warn);
                warnings.push(warn);
            }
        }
    }

    // 12. Send Completed Progress Update
    reporter.send_immediate(OperationStage::Completed, bytes_written, files_archived, None);

    Ok(ArchiveResult {
        output_path: strip_extended_prefix(&final_path),
        bytes_written,
        files_archived,
        total_files: scan_result.total_files,
        elapsed: start_time.elapsed(),
        warnings,
    })
}

/// Derives a valid archive name from sources or their parent directory.
fn derive_archive_name(sources: &[PathBuf], scan: &PreScanResult) -> Result<String, ArchiveError> {
    if sources.len() == 1 {
        let name = sources[0]
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Archive".to_string());
        if validate_archive_name(&name).is_ok() {
            return Ok(name);
        }
    } else if let Some(ref root) = scan.selection_root {
        if let Some(dir_name) = root.file_name().map(|n| n.to_string_lossy().to_string()) {
            if validate_archive_name(&dir_name).is_ok() {
                return Ok(dir_name);
            }
        }
    }

    Ok("Archive".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_create_archive_basic() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let folder = dir.path().join("TestFolder");
        std::fs::create_dir(&folder)?;

        let mut file1 = File::create(folder.join("test1.txt"))?;
        file1.write_all(b"Hello from test1")?;

        let mut file2 = File::create(folder.join("test2.txt"))?;
        file2.write_all(b"Hello from test2")?;

        let options = ZipOptions {
            predict_ext: false,
            ..Default::default()
        };

        let result = create_archive(&[folder.clone()], Some("OutputZip"), &options, None, None)?;

        assert!(result.output_path.exists());
        assert_eq!(result.files_archived, 2);
        assert_eq!(result.total_files, 2);
        assert!(result.bytes_written > 0);

        // Verify with ZipArchive
        let file = File::open(&result.output_path)?;
        let archive = zip::ZipArchive::new(file)?;
        assert_eq!(archive.len(), 2);
        Ok(())
    }

    #[test]
    fn test_create_archive_predict_osz() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let folder = dir.path().join("Beatmap");
        std::fs::create_dir(&folder)?;

        let mut osu = File::create(folder.join("map.osu"))?;
        osu.write_all(b"[General]\n")?;

        let options = ZipOptions {
            predict_ext: true,
            ..Default::default()
        };

        let result = create_archive(&[folder.clone()], None, &options, None, None)?;
        assert!(result.output_path.to_string_lossy().ends_with(".osz"));
        assert!(result.output_path.exists());
        Ok(())
    }
}
