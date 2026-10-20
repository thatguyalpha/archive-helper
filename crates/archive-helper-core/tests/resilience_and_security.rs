use archive_helper_core::error::ArchiveWarning;
use archive_helper_core::extract::{extract_archive, UnzipOptions};
use archive_helper_core::fs::normalize_extended_path;
use archive_helper_core::zip::{create_archive, ZipOptions};
use std::fs::File;
use std::io::Write;
use tempfile::tempdir;

#[test]
fn test_zip_slip_prevention() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let malicious_zip = dir.path().join("malicious.zip");

    // Manually construct a zip with a traversal entry
    {
        let file = File::create(&malicious_zip)?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        
        // zip crate or raw header with "../escaped.txt"
        zip.start_file("../escaped.txt", options)?;
        zip.write_all(b"malicious payload")?;
        zip.finish()?;
    }

    let extract_dir = dir.path().join("safe_target");
    std::fs::create_dir(&extract_dir)?;

    let unzip_opts = UnzipOptions {
        keep_top_dir: false,
        destination_dir: Some(extract_dir.clone()),
        ..Default::default()
    };

    let result = extract_archive(&malicious_zip, &unzip_opts, None, None);
    assert!(result.is_err(), "Zip Slip archive must be rejected");
    
    // Ensure file was not created outside target
    let escaped_file = dir.path().join("escaped.txt");
    assert!(!escaped_file.exists(), "Escaped file must not have been created on disk");
    Ok(())
}

#[test]
fn test_extended_path_normalization() {
    let path = std::path::Path::new("C:/Games/osu/Songs/VeryLongDirectoryName");
    let norm = normalize_extended_path(path);
    
    #[cfg(windows)]
    {
        let norm_str = norm.to_string_lossy();
        assert!(norm_str.starts_with(r"\\?\C:\") || norm_str.starts_with(r"\\?\UNC\"), "Should have extended length prefix");
    }
}

#[test]
fn test_resilient_skipping_unreadable_file() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempdir()?;
    let folder = dir.path().join("ResilientFolder");
    std::fs::create_dir(&folder)?;

    // Create readable file 1
    std::fs::write(folder.join("good1.txt"), b"good data 1")?;

    // Create readable file 2
    std::fs::write(folder.join("good2.txt"), b"good data 2")?;

    // Create locked/unreadable file on Windows (by opening with dwShareMode = 0)
    #[cfg(windows)]
    let _exclusive_file = {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .share_mode(0) // 0 means exclusive: NO other process can read/write/delete!
            .open(folder.join("locked.txt"))?
    };

    let options = ZipOptions {
        predict_ext: false,
        ..Default::default()
    };

    let result = create_archive(&[folder], Some("ResilientArchive"), &options, None, None)?;

    assert!(result.output_path.exists());
    assert_eq!(result.files_archived, 2); // 2 good files archived
    
    #[cfg(windows)]
    {
        // Should have captured 1 warning for the locked file!
        assert_eq!(result.warnings.len(), 1);
        match &result.warnings[0] {
            ArchiveWarning::FileReadSkipped { path, .. } => {
                assert!(path.to_string_lossy().contains("locked.txt"));
            }
            _ => panic!("Expected FileReadSkipped warning"),
        }
    }

    Ok(())
}
