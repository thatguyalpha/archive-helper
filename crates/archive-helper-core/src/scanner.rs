use crate::error::ArchiveError;
use crate::fs::{clean_path_components, filter_redundant_sources, is_same_or_child_path};
use std::path::PathBuf;
use std::time::SystemTime;

/// Predicted or designated archive extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ArchiveExtension {
    Zip,
    Osz,
    Osk,
}

impl ArchiveExtension {
    pub fn as_str(&self) -> &'static str {
        match self {
            ArchiveExtension::Zip => ".zip",
            ArchiveExtension::Osz => ".osz",
            ArchiveExtension::Osk => ".osk",
        }
    }
}

impl std::fmt::Display for ArchiveExtension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A discovered file or directory entry to be added to an archive.
#[derive(Debug, Clone)]
pub struct DiscoveredEntry {
    /// Full path on disk
    pub disk_path: PathBuf,
    /// POSIX path within the zip archive (e.g. "folder/sub/file.txt" or "folder/")
    pub archive_path: String,
    /// Whether this entry is a directory
    pub is_dir: bool,
    /// Uncompressed file size in bytes
    pub size: u64,
    /// Last modified time
    pub modified: Option<SystemTime>,
}

/// Comprehensive pre-scan metadata for archive creation.
#[derive(Debug, Clone)]
pub struct PreScanResult {
    /// Discovered entries in deterministic order
    pub entries: Vec<DiscoveredEntry>,
    /// Total bytes across all regular files
    pub total_bytes: u64,
    /// Total number of regular files
    pub total_files: usize,
    /// Total number of directories
    pub total_dirs: usize,
    /// Highest last modified timestamp found among entries
    pub max_modified: Option<SystemTime>,
    /// Predicted archive extension based on file types
    pub predicted_extension: ArchiveExtension,
    /// The computed root directory for the selection
    pub selection_root: Option<PathBuf>,
}

/// Pre-scans source items (files and directories) and prepares an entry manifest.
pub fn scan_sources(
    sources: &[PathBuf],
    predict_ext: bool,
) -> Result<PreScanResult, ArchiveError> {
    if sources.is_empty() {
        return Err(ArchiveError::EmptySource);
    }

    // Filter out redundant sources (e.g. child folder when parent is already selected)
    let filtered_sources = filter_redundant_sources(sources);
    if filtered_sources.is_empty() {
        return Err(ArchiveError::EmptySource);
    }

    // Verify all filtered sources exist
    for src in &filtered_sources {
        if !src.exists() {
            return Err(ArchiveError::SourceNotFound(src.clone()));
        }
    }

    let selection_root = compute_selection_root(&filtered_sources);
    let mut entries = Vec::new();
    let mut total_bytes = 0u64;
    let mut total_files = 0usize;
    let mut total_dirs = 0usize;
    let mut max_modified: Option<SystemTime> = None;
    let mut has_osu = false;
    let mut has_ini = false;

    let is_multi_source = filtered_sources.len() > 1;

    for src in &filtered_sources {
        let meta = match std::fs::metadata(src) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let file_name = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "entry".to_string());

        if meta.is_dir() {
            // Walk directory using walkdir for resilient, non-blocking traversal
            let walker = walkdir::WalkDir::new(src)
                .follow_links(false);

            // Add the root folder entry itself if multi-source
            if is_multi_source {
                let dir_archive_path = format!("{}/", file_name);
                entries.push(DiscoveredEntry {
                    disk_path: src.clone(),
                    archive_path: dir_archive_path,
                    is_dir: true,
                    size: 0,
                    modified: meta.modified().ok(),
                });
                total_dirs += 1;
            }

            for entry_res in walker {
                let entry = match entry_res {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                let entry_path = entry.path();
                // Skip the root folder itself if walking
                if entry_path == *src {
                    continue;
                }

                let entry_meta = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                let rel_path = match entry_path.strip_prefix(src) {
                    Ok(rel) => rel,
                    Err(_) => continue,
                };

                let rel_str = rel_path
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");

                let is_dir = entry_meta.is_dir();
                let archive_path = if is_multi_source {
                    if is_dir {
                        format!("{}/{}/", file_name, rel_str)
                    } else {
                        format!("{}/{}", file_name, rel_str)
                    }
                } else if is_dir {
                    format!("{}/", rel_str)
                } else {
                    rel_str.clone()
                };

                let size = if is_dir { 0 } else { entry_meta.len() };
                let modified = entry_meta.modified().ok();

                if let Some(m) = modified {
                    max_modified = Some(match max_modified {
                        Some(cur) => cur.max(m),
                        None => m,
                    });
                }

                if !is_dir {
                    total_bytes += size;
                    total_files += 1;

                    // Extension prediction check
                    if predict_ext {
                        let lower = rel_str.to_ascii_lowercase();
                        if lower.ends_with(".osu") {
                            has_osu = true;
                        } else if lower.ends_with(".ini") && !lower.ends_with("desktop.ini") {
                            has_ini = true;
                        }
                    }
                } else {
                    total_dirs += 1;
                }

                entries.push(DiscoveredEntry {
                    disk_path: entry_path.to_path_buf(),
                    archive_path,
                    is_dir,
                    size,
                    modified,
                });
            }
        } else {
            // Single file
            let size = meta.len();
            let modified = meta.modified().ok();
            if let Some(m) = modified {
                max_modified = Some(match max_modified {
                    Some(cur) => cur.max(m),
                    None => m,
                });
            }

            total_bytes += size;
            total_files += 1;

            if predict_ext {
                let lower = file_name.to_ascii_lowercase();
                if lower.ends_with(".osu") {
                    has_osu = true;
                } else if lower.ends_with(".ini") && !lower.ends_with("desktop.ini") {
                    has_ini = true;
                }
            }

            entries.push(DiscoveredEntry {
                disk_path: src.clone(),
                archive_path: file_name,
                is_dir: false,
                size,
                modified,
            });
        }
    }

    let predicted_extension = if predict_ext {
        if has_osu {
            ArchiveExtension::Osz
        } else if has_ini {
            ArchiveExtension::Osk
        } else {
            ArchiveExtension::Zip
        }
    } else {
        ArchiveExtension::Zip
    };

    Ok(PreScanResult {
        entries,
        total_bytes,
        total_files,
        total_dirs,
        max_modified,
        predicted_extension,
        selection_root,
    })
}

/// Computes the common ancestor directory for a slice of paths.
pub fn compute_selection_root(paths: &[PathBuf]) -> Option<PathBuf> {
    if paths.is_empty() {
        return None;
    }
    let first = clean_path_components(&paths[0]);
    let mut root = if first.is_dir() {
        first.parent().map(|p| p.to_path_buf())
    } else {
        first.parent().map(|p| p.to_path_buf())
    };

    for path in paths.iter().skip(1) {
        let clean = clean_path_components(path);
        let parent = clean.parent()?;
        while let Some(r) = &root {
            if is_same_or_child_path(parent, r) {
                break;
            }
            root = r.parent().map(|p| p.to_path_buf());
        }
        if root.is_none() {
            break;
        }
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_scan_and_predict_osu() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let beatmap_folder = dir.path().join("12345 Artist - Title");
        std::fs::create_dir(&beatmap_folder)?;

        let mut osu_file = File::create(beatmap_folder.join("song.osu"))?;
        osu_file.write_all(b"[General]\nMode: 0\n")?;
        osu_file.sync_all()?;
        drop(osu_file);

        let mut mp3_file = File::create(beatmap_folder.join("audio.mp3"))?;
        mp3_file.write_all(b"fake audio data")?;
        mp3_file.sync_all()?;
        drop(mp3_file);

        let result = scan_sources(&[beatmap_folder], true)?;
        assert_eq!(result.predicted_extension, ArchiveExtension::Osz);
        assert_eq!(result.total_files, 2);
        assert!(result.total_bytes > 0);
        Ok(())
    }

    #[test]
    fn test_scan_and_predict_osk() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let skin_folder = dir.path().join("CoolSkin");
        std::fs::create_dir(&skin_folder)?;

        let mut skin_ini = File::create(skin_folder.join("skin.ini"))?;
        skin_ini.write_all(b"[Skin]\nName: Cool\n")?;

        let result = scan_sources(&[skin_folder], true)?;
        assert_eq!(result.predicted_extension, ArchiveExtension::Osk);
        assert_eq!(result.total_files, 1);
        Ok(())
    }
}
