use crate::error::{ArchiveError, ArchiveWarning};
use crate::fs::{is_same_or_child_path, normalize_extended_path, safe_delete_file, strip_extended_prefix};
use crate::progress::{CancellationToken, OperationStage, ProgressReporter, ProgressUpdate};
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use zip::ZipArchive;

/// Options for archive extraction.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UnzipOptions {
    /// Delete original archive file after successful extraction.
    pub delete_original: bool,
    /// Extract into a subfolder named after the archive (keep_top_dir = true),
    /// or directly into the destination directory (keep_top_dir = false).
    pub keep_top_dir: bool,
    /// Destination directory (defaults to archive's parent directory).
    pub destination_dir: Option<PathBuf>,
    /// Preserve entry modified timestamps onto extracted files.
    pub keep_mtime: bool,
    /// Number of worker threads for parallel extraction (None = auto-detect CPU cores).
    pub max_threads: Option<usize>,
}

impl Default for UnzipOptions {
    fn default() -> Self {
        Self {
            delete_original: false,
            keep_top_dir: true,
            destination_dir: None,
            keep_mtime: true,
            max_threads: None,
        }
    }
}

/// Metadata summary of an extraction operation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExtractResult {
    /// Directory where contents were extracted
    pub output_dir: PathBuf,
    /// Total bytes extracted
    pub bytes_extracted: u64,
    /// Number of files extracted
    pub files_extracted: usize,
    /// Total files in archive
    pub total_files: usize,
    /// Elapsed time for extraction
    pub elapsed: Duration,
    /// Warnings captured during extraction
    pub warnings: Vec<ArchiveWarning>,
}

struct EntryDescriptor {
    index: usize,
    name: String,
    is_dir: bool,
    mtime: Option<filetime::FileTime>,
}

/// Extracts a `.zip`, `.osz`, or `.osk` archive using multi-threaded work-stealing parallel decompression.
pub fn extract_archive(
    archive_path: &Path,
    options: &UnzipOptions,
    cancellation: Option<&CancellationToken>,
    progress_tx: Option<crossbeam_channel::Sender<ProgressUpdate>>,
) -> Result<ExtractResult, ArchiveError> {
    let start_time = Instant::now();
    let archive_path = normalize_extended_path(archive_path);

    if !archive_path.exists() {
        return Err(ArchiveError::SourceNotFound(archive_path));
    }

    if let Some(token) = cancellation {
        token.check_cancelled()?;
    }

    let file = File::open(&archive_path)?;
    let mut zip = ZipArchive::new(BufReader::new(file))?;

    let archive_stem = archive_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Extracted".to_string());

    let base_dir = match &options.destination_dir {
        Some(dir) => dir.clone(),
        None => archive_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".")),
    };

    let target_dir = if options.keep_top_dir {
        base_dir.join(&archive_stem)
    } else {
        base_dir
    };

    std::fs::create_dir_all(&target_dir)?;

    // Scan entries upfront
    let mut total_bytes = 0u64;
    let mut total_files = 0usize;
    let mut descriptors = Vec::with_capacity(zip.len());

    for i in 0..zip.len() {
        let entry = zip.by_index_raw(i)?;
        let name = entry.name()?.to_string();
        let is_dir = entry.is_dir();
        let size = entry.size();

        // Zip Slip check
        let enclosed_path = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => {
                return Err(ArchiveError::ZipSlipDetected(name));
            }
        };

        let target_entry_path = target_dir.join(&enclosed_path);
        if !is_same_or_child_path(&target_entry_path, &target_dir) {
            return Err(ArchiveError::ZipSlipDetected(name));
        }

        let mtime = entry.last_modified().and_then(|dt| {
            let year = dt.year() as i32;
            let month = time::Month::try_from(dt.month()).ok()?;
            let date = time::Date::from_calendar_date(year, month, dt.day()).ok()?;
            let time = time::Time::from_hms(dt.hour(), dt.minute(), dt.second()).ok()?;
            let pdt = time::PrimitiveDateTime::new(date, time);
            Some(filetime::FileTime::from_unix_time(pdt.assume_utc().unix_timestamp(), 0))
        });

        if !is_dir {
            total_bytes += size;
            total_files += 1;
        }

        descriptors.push(EntryDescriptor {
            index: i,
            name,
            is_dir,
            mtime,
        });
    }

    // Pre-create all distinct directories upfront to prevent thread lock contention
    let mut distinct_dirs = HashSet::new();
    for desc in &descriptors {
        let dest = target_dir.join(&desc.name);
        if desc.is_dir {
            distinct_dirs.insert(dest);
        } else if let Some(parent) = dest.parent() {
            distinct_dirs.insert(parent.to_path_buf());
        }
    }

    for dir in distinct_dirs {
        std::fs::create_dir_all(dir)?;
    }

    let operation_id = format!("unzip-{}", fastrand::u64(..));
    let mut reporter = ProgressReporter::new(
        operation_id,
        archive_stem.clone(),
        total_bytes,
        total_files,
        progress_tx,
    );

    reporter.send_immediate(OperationStage::Extracting, 0, 0, None);

    // Parallel extraction of non-directory files
    let file_descriptors: Vec<_> = descriptors.into_iter().filter(|d| !d.is_dir).collect();
    let bytes_extracted = Arc::new(AtomicU64::new(0));
    let files_extracted = Arc::new(AtomicUsize::new(0));

    let archive_path_clone = archive_path.clone();
    let target_dir_clone = target_dir.clone();
    let keep_mtime = options.keep_mtime;

    // Use Rayon thread pool
    let thread_count = options
        .max_threads
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8));

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(thread_count)
        .build()
        .map_err(|e| ArchiveError::Other(e.to_string()))?;

    pool.install(|| -> Result<(), ArchiveError> {
        file_descriptors.par_iter().try_for_each(|desc| -> Result<(), ArchiveError> {
            if let Some(token) = cancellation {
                token.check_cancelled()?;
            }

            // Each worker opens its own read handle to avoid stream contention
            let file = File::open(&archive_path_clone)?;
            let mut local_zip = ZipArchive::new(BufReader::with_capacity(256 * 1024, file))?;
            let mut entry = local_zip.by_index(desc.index)?;

            let out_path = target_dir_clone.join(&desc.name);
            let mut out_file = File::create(&out_path)?;

            let mut buf = vec![0u8; 256 * 1024];
            loop {
                if let Some(token) = cancellation {
                    token.check_cancelled()?;
                }

                let n = entry.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                out_file.write_all(&buf[..n])?;
                bytes_extracted.fetch_add(n as u64, Ordering::Relaxed);
            }

            if keep_mtime {
                if let Some(mtime) = desc.mtime {
                    let _ = filetime::set_file_mtime(&out_path, mtime);
                }
            }

            files_extracted.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
    })?;

    let final_bytes = bytes_extracted.load(Ordering::SeqCst);
    let final_files = files_extracted.load(Ordering::SeqCst);

    let mut warnings = Vec::new();

    // Delete original archive if requested
    if options.delete_original {
        reporter.send_immediate(OperationStage::CleaningUp, final_bytes, final_files, None);
        if let Err(e) = safe_delete_file(&archive_path, 5, Duration::from_millis(150)) {
            let warn = ArchiveWarning::DeletionFailed {
                path: archive_path.clone(),
                reason: e.to_string(),
            };
            tracing::warn!("{}", warn);
            warnings.push(warn);
        }
    }

    reporter.send_immediate(OperationStage::Completed, final_bytes, final_files, None);

    Ok(ExtractResult {
        output_dir: strip_extended_prefix(&target_dir),
        bytes_extracted: final_bytes,
        files_extracted: final_files,
        total_files,
        elapsed: start_time.elapsed(),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zip::{create_archive, ZipOptions};
    use tempfile::tempdir;

    #[test]
    fn test_extract_archive_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let src_folder = dir.path().join("FolderToZip");
        std::fs::create_dir(&src_folder)?;
        std::fs::write(src_folder.join("data.txt"), b"Uncompressed file data")?;

        let zip_opts = ZipOptions::default();
        let zip_res = create_archive(&[src_folder], Some("ArchiveToExtract"), &zip_opts, None, None)?;

        let dest_extract = dir.path().join("ExtractedOutput");
        let unzip_opts = UnzipOptions {
            keep_top_dir: false,
            destination_dir: Some(dest_extract.clone()),
            ..Default::default()
        };

        let extract_res = extract_archive(&zip_res.output_path, &unzip_opts, None, None)?;
        assert_eq!(extract_res.files_extracted, 1);

        let extracted_file = dest_extract.join("data.txt");
        assert!(extracted_file.exists());
        let content = std::fs::read(&extracted_file)?;
        assert_eq!(content, b"Uncompressed file data");
        Ok(())
    }
}
