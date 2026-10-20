use crate::error::ArchiveError;
use std::fs::{File, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;

#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_SEQUENTIAL_SCAN, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
};

/// Opens a file for reading using the most permissive sharing mode available on the OS.
/// On Windows, this explicitly sets:
/// - `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`
/// - `FILE_FLAG_SEQUENTIAL_SCAN` for SSD/NVMe read-ahead cache optimization.
///
/// This prevents `ERROR_SHARING_VIOLATION` (0x20) when reading files currently open
/// in text editors, media players, or games.
pub fn open_resilient_read(path: &Path) -> std::io::Result<File> {
    let normalized = normalize_extended_path(path);
    let mut options = OpenOptions::new();
    options.read(true);

    #[cfg(windows)]
    {
        let share_mode = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
        options.share_mode(share_mode);
        options.custom_flags(FILE_FLAG_SEQUENTIAL_SCAN);
    }

    options.open(&normalized)
}

/// Normalizes a Windows path to use the Win32 extended-length prefix (`\\?\`),
/// resolving `.` and `..` segments and handling UNC network paths cleanly.
///
/// On non-Windows platforms, this cleans path components without modifying prefixes.
pub fn normalize_extended_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let cleaned = clean_path_components(path);
        let path_str = cleaned.to_string_lossy();

        // Already prefixed
        if path_str.starts_with(r"\\?\") || path_str.starts_with(r"\\.\") {
            return cleaned;
        }

        // UNC network path: \\server\share -> \\?\UNC\server\share
        if path_str.starts_with(r"\\") {
            let unc_tail = &path_str[2..];
            return PathBuf::from(format!(r"\\?\UNC\{}", unc_tail));
        }

        // Standard drive path: C:\path... -> \\?\C:\path...
        if path.is_absolute() {
            return PathBuf::from(format!(r"\\?\{}", path_str));
        }

        // For relative paths, canonicalize if possible or keep cleaned
        if let Ok(canonical) = dunce::canonicalize(&cleaned) {
            let canon_str = canonical.to_string_lossy();
            if !canon_str.starts_with(r"\\?\") {
                return PathBuf::from(format!(r"\\?\{}", canon_str));
            }
            return canonical;
        }

        cleaned
    }

    #[cfg(not(windows))]
    {
        clean_path_components(path)
    }
}

/// Strips the `\\?\` or `\\?\UNC\` extended length prefix for clean display in UI status bars.
pub fn strip_extended_prefix(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(unc) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{}", unc))
    } else if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        path.to_path_buf()
    }
}

/// Resolves `.` and `..` components purely lexically without accessing the filesystem.
pub fn clean_path_components(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            _ => out.push(comp.as_os_str()),
        }
    }
    out
}

/// Validates whether a proposed archive folder or file name is legal under Windows Win32 rules.
/// Checks reserved device names (CON, PRN, AUX, NUL, COM1-9, LPT1-9, etc.), forbidden characters,
/// and trailing dots or spaces.
pub fn validate_archive_name(name: &str) -> Result<(), ArchiveError> {
    if name.is_empty() || name.trim().is_empty() {
        return Err(ArchiveError::InvalidArchiveName("Name cannot be empty or whitespace".into()));
    }
    if name.len() > 251 {
        return Err(ArchiveError::InvalidArchiveName("Name exceeds maximum allowed length of 251 characters".into()));
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(ArchiveError::InvalidArchiveName("Name cannot end with a period or space".into()));
    }

    const FORBIDDEN_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
    if name.contains(|c: char| FORBIDDEN_CHARS.contains(&c) || (c as u32) < 32) {
        return Err(ArchiveError::InvalidArchiveName("Name contains invalid characters (< > : \" / \\ | ? * or control characters)".into()));
    }

    // Check reserved device stems
    let stem = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "CLOCK$", "CONIN$", "CONOUT$",
    ];
    if reserved.contains(&stem.as_str()) {
        return Err(ArchiveError::InvalidArchiveName(format!("'{}' is a reserved Windows device name", stem)));
    }

    // Check COM1-COM9 and LPT1-LPT9 (including superscripts 1-3)
    if stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) {
        let last_char = stem.chars().nth(3).unwrap();
        if "123456789¹²³".contains(last_char) {
            return Err(ArchiveError::InvalidArchiveName(format!("'{}' is a reserved Windows port name", stem)));
        }
    }

    Ok(())
}

/// Checks whether `child` is equal to or located inside `directory`.
pub fn is_same_or_child_path(child: &Path, directory: &Path) -> bool {
    let child_clean = clean_path_components(child);
    let dir_clean = clean_path_components(directory);

    if child_clean == dir_clean {
        return true;
    }

    if let Ok(rel) = child_clean.strip_prefix(&dir_clean) {
        !rel.as_os_str().is_empty()
    } else {
        false
    }
}

/// Filters out redundant descendant paths if their ancestor is already included in the list.
/// Also removes duplicate paths.
pub fn filter_redundant_sources(sources: &[PathBuf]) -> Vec<PathBuf> {
    let mut cleaned: Vec<PathBuf> = sources
        .iter()
        .map(|p| clean_path_components(p))
        .collect();

    cleaned.sort();
    cleaned.dedup();

    let mut filtered = Vec::new();
    for path in &cleaned {
        let has_ancestor = cleaned.iter().any(|other| {
            other != path && other.is_dir() && is_same_or_child_path(path, other)
        });
        if !has_ancestor {
            filtered.push(path.clone());
        }
    }
    filtered
}

/// Resilient file deletion that clears read-only attributes and retries on transient locks.
pub fn safe_delete_file(path: &Path, max_retries: u32, delay: Duration) -> std::io::Result<()> {
    let norm = normalize_extended_path(path);
    if !norm.exists() {
        return Ok(());
    }

    for attempt in 1..=max_retries {
        // Clear read-only attribute if set
        if let Ok(meta) = std::fs::metadata(&norm) {
            let mut perm = meta.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            if perm.readonly() {
                perm.set_readonly(false);
                let _ = std::fs::set_permissions(&norm, perm);
            }
        }

        match std::fs::remove_file(&norm) {
            Ok(_) => return Ok(()),
            Err(e) if attempt < max_retries => {
                tracing::debug!("Retry {}/{} deleting file '{}': {}", attempt, max_retries, path.display(), e);
                sleep(delay);
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Resilient recursive directory deletion that clears read-only attributes on all nested files.
pub fn safe_delete_dir_all(path: &Path, max_retries: u32, delay: Duration) -> std::io::Result<()> {
    let norm = normalize_extended_path(path);
    if !norm.exists() {
        return Ok(());
    }

    for attempt in 1..=max_retries {
        // Walk down and clear read-only attributes on all entries
        if let Ok(entries) = walkdir::WalkDir::new(&norm).into_iter().collect::<Result<Vec<_>, _>>() {
            for entry in entries {
                if let Ok(meta) = entry.metadata() {
                    let mut perm = meta.permissions();
                    if perm.readonly() {
                        perm.set_readonly(false);
                        let _ = std::fs::set_permissions(entry.path(), perm);
                    }
                }
            }
        }

        match std::fs::remove_dir_all(&norm) {
            Ok(_) => return Ok(()),
            Err(e) if attempt < max_retries => {
                tracing::debug!("Retry {}/{} deleting directory '{}': {}", attempt, max_retries, path.display(), e);
                sleep(delay);
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_archive_names() {
        assert!(validate_archive_name("MyBeatmap").is_ok());
        assert!(validate_archive_name("Skin_v2.1 (Final)").is_ok());
        assert!(validate_archive_name("CON").is_err());
        assert!(validate_archive_name("COM1").is_err());
        assert!(validate_archive_name("AUX.zip").is_err());
        assert!(validate_archive_name("bad:name").is_err());
        assert!(validate_archive_name("ends_with_dot.").is_err());
        assert!(validate_archive_name("ends_with_space ").is_err());
    }

    #[test]
    fn test_path_hierarchy() {
        let parent = Path::new("C:/Games/osu/Songs");
        let child = Path::new("C:/Games/osu/Songs/12345 Map");
        let outside = Path::new("C:/Games/osu/Skins");

        assert!(is_same_or_child_path(child, parent));
        assert!(is_same_or_child_path(parent, parent));
        assert!(!is_same_or_child_path(outside, parent));
    }
}
