use crate::error::ArchiveError;
use crate::extract::{extract_archive, ExtractResult, UnzipOptions};
use crate::progress::{CancellationToken, ProgressUpdate};
use crate::zip::{create_archive, ArchiveResult, ZipOptions};
use crossbeam_channel::Receiver;
use rayon::ThreadPoolBuilder;
use std::path::PathBuf;

/// A task to be executed within a batch operation.
#[derive(Debug, Clone)]
pub enum BatchTask {
    Zip {
        id: String,
        sources: Vec<PathBuf>,
        custom_name: Option<String>,
        options: ZipOptions,
    },
    Unzip {
        id: String,
        archive_path: PathBuf,
        options: UnzipOptions,
    },
}

impl BatchTask {
    pub fn id(&self) -> &str {
        match self {
            BatchTask::Zip { id, .. } => id,
            BatchTask::Unzip { id, .. } => id,
        }
    }
}

/// The result of a single task executed in a batch.
#[derive(Debug)]
pub enum BatchTaskResult {
    Zip(Result<ArchiveResult, ArchiveError>),
    Unzip(Result<ExtractResult, ArchiveError>),
}

/// Coordinates batch processing across multiple archives with controlled concurrency.
pub struct BatchCoordinator {
    max_concurrent_jobs: usize,
}

impl BatchCoordinator {
    /// Creates a new coordinator.
    /// `max_concurrent_jobs`: Number of simultaneous archive jobs (e.g. 2-4 recommended for SSDs).
    pub fn new(max_concurrent_jobs: usize) -> Self {
        let max_jobs = if max_concurrent_jobs == 0 {
            2
        } else {
            max_concurrent_jobs
        };
        Self {
            max_concurrent_jobs: max_jobs,
        }
    }

    /// Executes a list of tasks using a work-stealing thread pool, streaming progress updates
    /// to the returned crossbeam receiver.
    pub fn execute_batch(
        &self,
        tasks: Vec<BatchTask>,
        cancellation: Option<CancellationToken>,
    ) -> (Receiver<ProgressUpdate>, Receiver<(String, BatchTaskResult)>) {
        let (progress_tx, progress_rx) = crossbeam_channel::unbounded();
        let (result_tx, result_rx) = crossbeam_channel::unbounded();

        let pool = ThreadPoolBuilder::new()
            .num_threads(self.max_concurrent_jobs)
            .build()
            .expect("Failed to initialize batch thread pool");

        let cancellation = cancellation.unwrap_or_default();

        std::thread::spawn(move || {
            pool.install(|| {
                use rayon::prelude::*;
                tasks.into_par_iter().for_each(|task| {
                    let task_id = task.id().to_string();
                    let task_cancel = cancellation.clone();
                    let tx = progress_tx.clone();

                    let res = match task {
                        BatchTask::Zip {
                            sources,
                            custom_name,
                            options,
                            ..
                        } => {
                            let r = create_archive(
                                &sources,
                                custom_name.as_deref(),
                                &options,
                                Some(&task_cancel),
                                Some(tx),
                            );
                            BatchTaskResult::Zip(r)
                        }
                        BatchTask::Unzip {
                            archive_path,
                            options,
                            ..
                        } => {
                            let r = extract_archive(
                                &archive_path,
                                &options,
                                Some(&task_cancel),
                                Some(tx),
                            );
                            BatchTaskResult::Unzip(r)
                        }
                    };

                    let _ = result_tx.send((task_id, res));
                });
            });
        });

        (progress_rx, result_rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_batch_execution() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let folder1 = dir.path().join("Folder1");
        let folder2 = dir.path().join("Folder2");
        std::fs::create_dir(&folder1)?;
        std::fs::create_dir(&folder2)?;
        std::fs::write(folder1.join("file.txt"), b"folder 1 data")?;
        std::fs::write(folder2.join("file.txt"), b"folder 2 data")?;

        let tasks = vec![
            BatchTask::Zip {
                id: "job-1".into(),
                sources: vec![folder1],
                custom_name: Some("Folder1Archive".into()),
                options: ZipOptions::default(),
            },
            BatchTask::Zip {
                id: "job-2".into(),
                sources: vec![folder2],
                custom_name: Some("Folder2Archive".into()),
                options: ZipOptions::default(),
            },
        ];

        let coordinator = BatchCoordinator::new(2);
        let (_progress_rx, result_rx) = coordinator.execute_batch(tasks, None);

        let mut completed = 0;
        while let Ok((id, res)) = result_rx.recv() {
            match res {
                BatchTaskResult::Zip(Ok(arch)) => {
                    assert!(arch.output_path.exists());
                    completed += 1;
                }
                BatchTaskResult::Zip(Err(e)) => panic!("Job {} failed with error: {:?}", id, e),
                BatchTaskResult::Unzip(Err(e)) => panic!("Job {} unzip failed with error: {:?}", id, e),
                _ => panic!("Job {} failed", id),
            }
        }

        assert_eq!(completed, 2);
        Ok(())
    }
}
