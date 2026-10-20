use crate::error::ArchiveError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Stages of an archive operation lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum OperationStage {
    Scanning,
    Archiving,
    Extracting,
    Finalizing,
    CleaningUp,
    Completed,
    Failed,
    Cancelled,
}

/// Granular progress update for UI and worker consumption.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProgressUpdate {
    /// Identifier for this operation/queue item
    pub operation_id: String,
    /// Display name of the item being archived or extracted
    pub item_name: String,
    /// Current operation stage
    pub stage: OperationStage,
    /// Bytes processed so far
    pub bytes_processed: u64,
    /// Total bytes expected
    pub total_bytes: u64,
    /// Number of files processed so far
    pub files_processed: usize,
    /// Total number of files
    pub total_files: usize,
    /// Name of the file currently being processed
    pub current_file: Option<String>,
    /// Instantaneous throughput in bytes per second
    pub bytes_per_sec: f64,
}

impl ProgressUpdate {
    /// Percentage complete (0.0 to 100.0) based on bytes.
    pub fn percent(&self) -> f32 {
        if self.total_bytes == 0 {
            if self.stage == OperationStage::Completed {
                100.0
            } else {
                0.0
            }
        } else {
            ((self.bytes_processed as f64 / self.total_bytes as f64) * 100.0).clamp(0.0, 100.0) as f32
        }
    }
}

/// Thread-safe cancellation token.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Sets the token to cancelled state.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Returns true if cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Helper to return `Err(ArchiveError::Cancelled)` if cancelled.
    pub fn check_cancelled(&self) -> Result<(), ArchiveError> {
        if self.is_cancelled() {
            Err(ArchiveError::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Throttled progress reporter that prevents UI thread message flooding on fast NVMe drives.
pub struct ProgressReporter {
    operation_id: String,
    item_name: String,
    total_bytes: u64,
    total_files: usize,
    last_report: Instant,
    report_interval: Duration,
    last_bytes: u64,
    sender: Option<crossbeam_channel::Sender<ProgressUpdate>>,
}

impl ProgressReporter {
    /// Creates a new reporter with a default 40ms (~25 Hz) throttle interval.
    pub fn new(
        operation_id: String,
        item_name: String,
        total_bytes: u64,
        total_files: usize,
        sender: Option<crossbeam_channel::Sender<ProgressUpdate>>,
    ) -> Self {
        Self {
            operation_id,
            item_name,
            total_bytes,
            total_files,
            last_report: Instant::now() - Duration::from_millis(100),
            report_interval: Duration::from_millis(40),
            last_bytes: 0,
            sender,
        }
    }

    /// Sends a progress report immediately without checking the throttle interval.
    pub fn send_immediate(
        &mut self,
        stage: OperationStage,
        bytes_processed: u64,
        files_processed: usize,
        current_file: Option<String>,
    ) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_report).as_secs_f64();
        let bytes_delta = bytes_processed.saturating_sub(self.last_bytes);
        let bytes_per_sec = if elapsed > 0.001 {
            bytes_delta as f64 / elapsed
        } else {
            0.0
        };

        self.last_report = now;
        self.last_bytes = bytes_processed;

        if let Some(ref tx) = self.sender {
            let _ = tx.send(ProgressUpdate {
                operation_id: self.operation_id.clone(),
                item_name: self.item_name.clone(),
                stage,
                bytes_processed,
                total_bytes: self.total_bytes,
                files_processed,
                total_files: self.total_files,
                current_file,
                bytes_per_sec,
            });
        }
    }

    /// Sends a progress report only if the configured throttle duration (e.g. 40ms) has elapsed.
    pub fn send_throttled(
        &mut self,
        stage: OperationStage,
        bytes_processed: u64,
        files_processed: usize,
        current_file: Option<String>,
    ) {
        if self.last_report.elapsed() >= self.report_interval {
            self.send_immediate(stage, bytes_processed, files_processed, current_file);
        }
    }
}
