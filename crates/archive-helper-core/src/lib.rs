//! # archive-helper-core
//!
//! High-performance, resilient, unelevated zero-compression archive engine.
//!
//! Key architectural features:
//! - **Atomic File Writes with RAII Cleanup**: Archives are written to temporary files and atomically committed upon completion. Unhandled errors, cancellations, or panics cleanly delete temporary files so no 0-byte or corrupted archives ever remain on disk.
//! - **Resilient Unelevated Reading**: Uses native Windows `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE` flags so files currently open in media players, games, or editors can still be archived.
//! - **Graceful File Skip**: Locked or inaccessible files emit non-fatal warnings and are skipped without aborting multi-gigabyte jobs.
//! - **Windows Extended-Length Path Normalization**: Safely handles paths exceeding the 260-character `MAX_PATH` limit (`\\?\` prefixing).
//! - **Zero-Compression Sequential Streaming**: Uses standard ZIP Data Descriptors without backward seeks, saturating NVMe and SSD drive write caches.
//! - **Parallel Directory Pre-scanning**: Fast multi-threaded discovery of total bytes, file counts, and extension prediction (.zip, .osz, .osk).
//! - **Multi-Threaded Extraction**: Work-stealing decompression with `rayon` and pre-created directory hierarchies to eliminate lock contention.
//! - **Granular Throttled Progress**: Channel-based updates (bytes, files, current item, throughput) with 40ms throttling to protect UI render threads.

pub mod atomic;
pub mod batch;
pub mod error;
pub mod extract;
pub mod fs;
pub mod progress;
pub mod scanner;
pub mod zip;

// Re-exports of primary types
pub use atomic::AtomicFileWriter;
pub use batch::{BatchCoordinator, BatchTask, BatchTaskResult};
pub use error::{ArchiveError, ArchiveWarning};
pub use extract::{extract_archive, inspect_zip, ExtractResult, UnzipOptions};
pub use fs::{
    filter_redundant_sources, is_same_or_child_path, normalize_extended_path, open_resilient_read,
    safe_delete_dir_all, safe_delete_file, strip_extended_prefix, validate_archive_name,
};
pub use progress::{CancellationToken, OperationStage, ProgressReporter, ProgressUpdate};
pub use scanner::{
    compute_selection_root, scan_sources, ArchiveExtension, DiscoveredEntry, PreScanResult,
};
pub use zip::{create_archive, system_time_to_zip_datetime, ArchiveResult, ZipOptions};
