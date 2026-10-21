use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

/// RAII wrapper that manages writing to an atomic temporary file.
/// If an error, panic, or cancellation occurs prior to calling `commit()`,
/// the destructor automatically closes the handle and deletes the temporary file,
/// guaranteeing that no 0-byte or corrupt files remain on disk.
pub struct AtomicFileWriter {
    temp_path: PathBuf,
    target_path: PathBuf,
    writer: Option<BufWriter<File>>,
    committed: bool,
}

impl AtomicFileWriter {
    /// Default buffer size for sequential archive streaming (1 MiB).
    pub const DEFAULT_BUFFER_SIZE: usize = 1024 * 1024;

    /// Creates a new `AtomicFileWriter` in the same parent directory as `target_path`.
    pub fn new(target_path: &Path) -> io::Result<Self> {
        Self::with_buffer_size(target_path, Self::DEFAULT_BUFFER_SIZE)
    }

    /// Creates a new `AtomicFileWriter` with a custom buffer size.
    pub fn with_buffer_size(target_path: &Path, buffer_size: usize) -> io::Result<Self> {
        let target_path = target_path.to_path_buf();
        let parent_dir = target_path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Target path has no parent directory",
            )
        })?;

        // Ensure parent directory exists
        std::fs::create_dir_all(parent_dir)?;

        // Generate a unique hidden temporary file in the same directory
        let temp_filename = format!(".archive-helper-{}.tmp", fastrand::u64(..));
        let temp_path = parent_dir.join(temp_filename);

        // Open temporary file exclusively
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;

        // Register active temporary file in the journal for crash/kill resilience
        crate::journal::register_temp_file(&temp_path);

        let writer = BufWriter::with_capacity(buffer_size, file);

        Ok(Self {
            temp_path,
            target_path,
            writer: Some(writer),
            committed: false,
        })
    }

    /// Returns a reference to the temporary file path.
    pub fn temp_path(&self) -> &Path {
        &self.temp_path
    }

    /// Returns a reference to the target destination path.
    pub fn target_path(&self) -> &Path {
        &self.target_path
    }

    /// Flushes all remaining buffers, closes the file handle, and atomically moves
    /// the temporary file to the final destination path.
    pub fn commit(mut self) -> io::Result<PathBuf> {
        if let Some(mut w) = self.writer.take() {
            w.flush()?;
        }
        // File handle is now dropped/closed, releasing any Windows OS file lock

        // Ensure destination is writable if it already exists
        if let Ok(meta) = std::fs::metadata(&self.target_path) {
            let mut perm = meta.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            if perm.readonly() {
                perm.set_readonly(false);
                let _ = std::fs::set_permissions(&self.target_path, perm);
            }
        }

        // Atomically replace destination file
        std::fs::rename(&self.temp_path, &self.target_path)?;
        self.committed = true;
        crate::journal::unregister_temp_file(&self.temp_path);
        Ok(self.target_path.clone())
    }

    /// Explicitly aborts and cleans up the temporary file immediately.
    pub fn abort(mut self) {
        self.cleanup();
    }

    fn cleanup(&mut self) {
        if !self.committed {
            // Close writer handle first
            self.writer.take();
            // Delete temp file
            if self.temp_path.exists() {
                let _ = std::fs::remove_file(&self.temp_path);
            }
            crate::journal::unregister_temp_file(&self.temp_path);
        }
    }
}

impl Write for AtomicFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match &mut self.writer {
            Some(w) => w.write(buf),
            None => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "Writer already closed",
            )),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.writer {
            Some(w) => w.flush(),
            None => Ok(()),
        }
    }
}

impl Drop for AtomicFileWriter {
    fn drop(&mut self) {
        self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_atomic_write_commit() -> io::Result<()> {
        let dir = tempdir()?;
        let target = dir.path().join("output.zip");

        let mut writer = AtomicFileWriter::new(&target)?;
        writer.write_all(b"PK\x03\x04testdata")?;
        let output = writer.commit()?;

        assert_eq!(output, target);
        assert!(target.exists());
        let content = std::fs::read(&target)?;
        assert_eq!(content, b"PK\x03\x04testdata");
        Ok(())
    }

    #[test]
    fn test_atomic_write_abort_on_drop() -> io::Result<()> {
        let dir = tempdir()?;
        let target = dir.path().join("aborted.zip");

        {
            let mut writer = AtomicFileWriter::new(&target)?;
            writer.write_all(b"partial corrupted data")?;
            // Do NOT call commit() -> dropped here!
        }

        // Neither the target nor the temp file should exist
        assert!(
            !target.exists(),
            "Target file must not exist if uncommitted"
        );
        let remaining_files: Vec<_> = std::fs::read_dir(dir.path())?.collect();
        assert!(
            remaining_files.is_empty(),
            "Temp file should be cleanly removed on drop"
        );
        Ok(())
    }
}
