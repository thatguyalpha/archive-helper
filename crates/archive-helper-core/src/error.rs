use std::path::PathBuf;
use thiserror::Error;

/// Core errors that can occur during archive creation, scanning, or extraction.
#[derive(Debug, Error)]
pub enum ArchiveError {
    #[error("Invalid archive name: {0}")]
    InvalidArchiveName(String),

    #[error("No source items were provided for archiving")]
    EmptySource,

    #[error("Source item not found: {0:?}")]
    SourceNotFound(PathBuf),

    #[error("Duplicate item name or conflict detected: {0}")]
    DuplicateItemName(String),

    #[error("Circular dependency: output directory '{output_path:?}' cannot be inside source item '{source_path:?}'")]
    CircularOutputTarget {
        source_path: PathBuf,
        output_path: PathBuf,
    },

    #[error("Archive output cannot overwrite a source file: {0:?}")]
    TargetOverwritesSource(PathBuf),

    #[error("Zip Slip vulnerability detected in archive entry: '{0}' attempts path traversal")]
    ZipSlipDetected(String),

    #[error("Operation was cancelled by user")]
    Cancelled,

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Zip format error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("Other error: {0}")]
    Other(String),
}

/// Non-fatal warnings captured during archiving/extraction that permit
/// the operation to continue resiliently without aborting.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ArchiveWarning {
    /// A source file could not be read (e.g. locked exclusively by another process, or permission denied).
    FileReadSkipped {
        path: PathBuf,
        reason: String,
    },
    /// An original source file or folder could not be deleted after successful archiving.
    DeletionFailed {
        path: PathBuf,
        reason: String,
    },
    /// Metadata (such as Last Modified Time) could not be applied to an entry or archive.
    MetadataFailed {
        path: PathBuf,
        reason: String,
    },
    /// General warning message.
    General(String),
}

impl std::fmt::Display for ArchiveWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArchiveWarning::FileReadSkipped { path, reason } => {
                write!(f, "Skipped locked/unreadable file '{}': {}", path.display(), reason)
            }
            ArchiveWarning::DeletionFailed { path, reason } => {
                write!(f, "Could not delete original '{}': {}", path.display(), reason)
            }
            ArchiveWarning::MetadataFailed { path, reason } => {
                write!(f, "Failed to apply metadata on '{}': {}", path.display(), reason)
            }
            ArchiveWarning::General(msg) => write!(f, "{}", msg),
        }
    }
}
