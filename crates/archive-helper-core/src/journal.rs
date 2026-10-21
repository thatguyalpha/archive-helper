use crate::fs::{normalize_extended_path, safe_delete_file, strip_extended_prefix};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

static JOURNAL_LOCK: Mutex<()> = Mutex::new(());

/// Returns whether a given path conforms to the ArchiveHelper temporary file pattern:
/// `.archive-helper-<id>.tmp`.
pub fn is_archive_helper_temp_file(path: &Path) -> bool {
    let stripped = strip_extended_prefix(path);
    let file_name = match stripped.file_name().and_then(|n| n.to_str()) {
        Some(name) => name,
        None => return false,
    };
    file_name.starts_with(".archive-helper-") && file_name.ends_with(".tmp")
}

/// Resolves the default system path to the active temporary files journal.
pub fn get_journal_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Ok(app_data) = std::env::var("APPDATA") {
            return PathBuf::from(app_data)
                .join("ArchiveHelper")
                .join("active_tmp_files.json");
        }
    }

    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".archive-helper")
            .join("active_tmp_files.json");
    }

    std::env::temp_dir().join("archive_helper_active_tmp.json")
}

fn load_journal_entries(journal_path: &Path) -> Vec<PathBuf> {
    if !journal_path.exists() {
        return Vec::new();
    }
    match fs::read_to_string(journal_path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn save_journal_entries(journal_path: &Path, entries: &[PathBuf]) {
    if entries.is_empty() {
        if journal_path.exists() {
            let _ = fs::remove_file(journal_path);
        }
        return;
    }

    if let Some(parent) = journal_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    if let Ok(json) = serde_json::to_string_pretty(entries) {
        let _ = fs::write(journal_path, json);
    }
}

fn normalize_for_comparison(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Registers an active temporary file with the journal at a custom path.
pub fn register_temp_file_at(journal_path: &Path, path: &Path) {
    let _guard = JOURNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut entries = load_journal_entries(journal_path);

    let target_norm = normalize_for_comparison(path);
    if !entries
        .iter()
        .any(|e| normalize_for_comparison(e) == target_norm)
    {
        entries.push(path.to_path_buf());
        save_journal_entries(journal_path, &entries);
    }
}

/// Registers an active temporary file with the default persistent journal.
/// Called immediately when an atomic temporary file is created.
pub fn register_temp_file(path: &Path) {
    register_temp_file_at(&get_journal_path(), path);
}

/// Unregisters an active temporary file from the journal at a custom path.
pub fn unregister_temp_file_at(journal_path: &Path, path: &Path) {
    let _guard = JOURNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut entries = load_journal_entries(journal_path);

    let target_norm = normalize_for_comparison(path);
    let original_len = entries.len();
    entries.retain(|e| normalize_for_comparison(e) != target_norm);

    if entries.len() != original_len {
        save_journal_entries(journal_path, &entries);
    }
}

/// Unregisters an active temporary file from the default journal.
/// Called upon atomic commit or clean abort/destruction.
pub fn unregister_temp_file(path: &Path) {
    unregister_temp_file_at(&get_journal_path(), path);
}

/// Sweeps the journal at the specified path and deletes all orphaned temporary files.
pub fn cleanup_orphaned_temp_files_at(journal_path: &Path) -> Vec<PathBuf> {
    let _guard = JOURNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let entries = load_journal_entries(journal_path);
    if entries.is_empty() {
        return Vec::new();
    }

    let mut remaining = Vec::new();
    let mut cleaned = Vec::new();

    for path in entries {
        // Strict security assertion: NEVER delete any file that doesn't strictly match the temp pattern
        if !is_archive_helper_temp_file(&path) {
            tracing::warn!(
                "Ignoring untrusted journal entry '{}' (does not match .archive-helper-*.tmp)",
                path.display()
            );
            continue;
        }

        let norm = normalize_extended_path(&path);
        if !norm.exists() {
            // File is already gone, no need to keep in journal
            continue;
        }

        // Attempt resilient deletion (retrying up to 3 times with 100ms backoff)
        match safe_delete_file(&path, 3, Duration::from_millis(100)) {
            Ok(_) => {
                tracing::info!("Cleaned up orphaned temporary file: {}", path.display());
                cleaned.push(path);
            }
            Err(e) => {
                tracing::warn!(
                    "Could not remove orphaned temporary file '{}' (may still be locked by another process): {}",
                    path.display(),
                    e
                );
                // Keep locked files in the journal so they can be cleaned on subsequent runs
                remaining.push(path);
            }
        }
    }

    save_journal_entries(journal_path, &remaining);
    cleaned
}

/// Sweeps the default persistent journal and deletes all orphaned temporary files that remain
/// from previous crashed, interrupted, or forcibly terminated sessions.
///
/// Returns the list of successfully cleaned temporary file paths.
pub fn cleanup_orphaned_temp_files() -> Vec<PathBuf> {
    cleanup_orphaned_temp_files_at(&get_journal_path())
}

/// Scans a directory for any un-journaled `.archive-helper-*.tmp` files and safely removes them.
/// Useful for sweeping directories containing leftover files from older sessions.
pub fn cleanup_temp_files_in_dir(dir: &Path) -> Vec<PathBuf> {
    let mut cleaned = Vec::new();
    let read_dir = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return cleaned,
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_file()
            && is_archive_helper_temp_file(&path)
            && safe_delete_file(&path, 2, Duration::from_millis(50)).is_ok()
        {
            cleaned.push(path);
        }
    }

    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_is_archive_helper_temp_file() {
        assert!(is_archive_helper_temp_file(Path::new(
            ".archive-helper-123456789.tmp"
        )));
        assert!(is_archive_helper_temp_file(Path::new(
            "C:/games/.archive-helper-999.tmp"
        )));
        assert!(is_archive_helper_temp_file(Path::new(
            r"\\?\D:\folder\.archive-helper-42.tmp"
        )));

        assert!(!is_archive_helper_temp_file(Path::new("normal_file.tmp")));
        assert!(!is_archive_helper_temp_file(Path::new(
            ".archive-helper-123.zip"
        )));
        assert!(!is_archive_helper_temp_file(Path::new("important_doc.pdf")));
    }

    #[test]
    fn test_journal_registration_lifecycle() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let journal_file = dir.path().join("active_tmp_files.json");

        let fake_temp1 = dir.path().join(".archive-helper-111.tmp");
        let fake_temp2 = dir.path().join(".archive-helper-222.tmp");
        fs::write(&fake_temp1, b"temp data 1")?;
        fs::write(&fake_temp2, b"temp data 2")?;

        // 1. Register files
        register_temp_file_at(&journal_file, &fake_temp1);
        register_temp_file_at(&journal_file, &fake_temp2);

        let entries = load_journal_entries(&journal_file);
        assert_eq!(entries.len(), 2);

        // 2. Unregister one file
        unregister_temp_file_at(&journal_file, &fake_temp1);
        let entries = load_journal_entries(&journal_file);
        assert_eq!(entries.len(), 1);

        // 3. Cleanup remaining orphaned file
        let cleaned = cleanup_orphaned_temp_files_at(&journal_file);
        assert_eq!(cleaned.len(), 1);
        assert!(
            !fake_temp2.exists(),
            "Orphaned temp file 2 must have been deleted"
        );

        // Journal should now be cleaned/empty
        let entries = load_journal_entries(&journal_file);
        assert!(entries.is_empty());

        Ok(())
    }

    #[test]
    fn test_cleanup_security_ignores_non_temp_files() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let journal_file = dir.path().join("active_tmp_files.json");

        let important_file = dir.path().join("user_document.txt");
        fs::write(&important_file, b"cannot be deleted")?;

        // Write important file directly to journal
        save_journal_entries(&journal_file, &[important_file.clone()]);

        let cleaned = cleanup_orphaned_temp_files_at(&journal_file);
        assert!(cleaned.is_empty());
        assert!(
            important_file.exists(),
            "Non-matching file must never be deleted"
        );

        Ok(())
    }

    #[test]
    fn test_cleanup_temp_files_in_dir() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let temp1 = dir.path().join(".archive-helper-555.tmp");
        let temp2 = dir.path().join(".archive-helper-666.tmp");
        let regular = dir.path().join("keep_me.txt");

        fs::write(&temp1, b"abandoned 1")?;
        fs::write(&temp2, b"abandoned 2")?;
        fs::write(&regular, b"real data")?;

        let cleaned = cleanup_temp_files_in_dir(dir.path());
        assert_eq!(cleaned.len(), 2);
        assert!(!temp1.exists());
        assert!(!temp2.exists());
        assert!(regular.exists(), "Regular file must remain untouched");

        Ok(())
    }
}
