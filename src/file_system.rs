#![allow(dead_code)]
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use crate::err;

#[derive(Debug, Clone)]
pub struct ProjectRoot(PathBuf);
#[derive(Debug, Clone)]
pub struct DataFolder(PathBuf);
#[derive(Debug, Clone)]
pub struct SentinelFile(PathBuf);

impl ProjectRoot {
    pub fn get(&self) -> PathBuf {
        self.0.clone()
    }
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        assert!(path.is_dir());
        Self(path)
    }
    pub fn data_folder(&self) -> DataFolder {
        DataFolder(self.0.join("checkpoint_data"))
    }
    pub fn untracked(&self) -> PathBuf {
        self.get().join("untracked_files")
    }

    pub fn is_trackable(&self, path: &Path) -> bool {
        // Skip internal tracking database files/folders
        if path.starts_with(self.data_folder().get()) || path.starts_with(self.untracked()) {
            return false;
        }

        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            return false;
        }

        true
    }
}

impl DataFolder {
    pub fn get(&self) -> PathBuf {
        self.0.clone()
    }
    pub fn sentinel(&self) -> SentinelFile {
        SentinelFile(self.0.join("project.checkpoint"))
    }
    pub fn project_root(&self) -> ProjectRoot {
        ProjectRoot(
            self.0
                .parent()
                .expect("Found DataFolder without parent")
                .to_path_buf(),
        )
    }
    pub fn data_json(&self) -> PathBuf {
        self.0.join("data.json")
    }
    pub fn data_tmp(&self) -> PathBuf {
        self.0.join("data.tmp")
    }
    pub fn wal(&self) -> PathBuf {
        self.0.join("restore.wal")
    }
}

impl SentinelFile {
    pub fn get(&self) -> PathBuf {
        self.0.clone()
    }
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        assert!(path.extension() == Some(OsStr::new("checkpoint")));
        SentinelFile(path)
    }

    pub fn data_folder(&self) -> DataFolder {
        DataFolder(
            self.0
                .parent()
                .expect("Found Sentinel without parent")
                .to_path_buf(),
        )
    }
    pub fn project_root(&self) -> ProjectRoot {
        self.data_folder().project_root()
    }
}

/// Returns the ID of a failed commit if the last restore failed. Returns None otherwise.
pub fn get_failed_restore_id(root: &ProjectRoot) -> Option<usize> {
    let log_path = root.data_folder().wal();
    std::fs::read_to_string(&log_path)
        .ok()
        .and_then(|content| content.trim().parse::<usize>().ok())
}

pub fn init_project(root: &ProjectRoot) -> Result<(), err::Init> {
    let sentinel = root.data_folder().sentinel().get();
    let data_folder = root.data_folder().get();
    let readme = data_folder.join("readme.txt");
    let untracked = root.untracked();
    let untracked_readme = root.untracked().join("readme.txt");

    if std::fs::exists(sentinel.clone()).map_err(|e| err::Init::Io(sentinel.clone(), e))? {
        return Err(err::Init::ProjectAlreadyExists);
    }

    std::fs::create_dir_all(data_folder.clone()).map_err(|e| err::Init::Io(data_folder, e))?;
    std::fs::create_dir_all(untracked.clone()).map_err(|e| err::Init::Io(untracked, e))?;
    std::fs::write(sentinel.clone(), "").map_err(|e| err::Init::Io(sentinel, e))?;
    std::fs::write(
        readme.clone(),
        "This folder is used by CheckpointPro to create and load checkpoints.\nModifying these files will probably prevent CheckpointPro from working correctly.",
    )
    .map_err(|e| err::Init::Io(readme, e))?;
    std::fs::write(
        untracked_readme.clone(),
        "CheckpointPro will ignore this folder when creating and restoring checkpoints.\nYou may want to store very large files like high-resolution images, video, or datasets here so they can live in your project folder without slowing down your checkpoints.",
    )
    .map_err(|e| err::Init::Io(untracked_readme, e))?;

    Ok(())
}
