use serde_with::serde_as;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs::OpenOptions, io::ErrorKind, path::PathBuf};

pub(crate) use chrono::{DateTime, Local};

use crate::{
    err,
    file_system::{ProjectRoot, SentinelFile},
    licensing::{Registration, RegistrationInfo},
    ui::UIData,
    utilities::{self, FileType, classify_file, decode_utf16},
};

#[serde_as]
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Commit {
    pub message: String,
    #[serde_as(as = "Vec<(_, serde_with::base64::Base64)>")]
    pub files: Vec<(PathBuf, FileHash)>,
    pub description: String,
    pub timestamp: DateTime<Local>,
    pub additions: usize,
    pub deletions: usize,
}

impl Commit {
    pub fn new(
        message: String,
        description: String,
        files: Vec<(PathBuf, FileHash)>,
        additions: usize,
        deletions: usize,
    ) -> Self {
        Self {
            message,
            description,
            files,
            timestamp: Local::now(),
            additions,
            deletions,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitResult {
    Success,
    NoOp,
}

pub type FileHash = [u8; 32];

#[serde_as]
#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct FileData(#[serde_as(as = "serde_with::base64::Base64")] pub Vec<u8>);

#[serde_as]
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct Repo {
    #[serde_as(as = "HashMap<serde_with::base64::Base64, _>")]
    pub file_data: HashMap<FileHash, FileData>,
    pub commits: Vec<Commit>,
    pub current_checkpoint: Option<usize>,
}

impl Repo {
    pub fn new() -> Self {
        Self {
            commits: vec![],
            file_data: HashMap::new(),
            current_checkpoint: None,
        }
    }

    pub fn last_commit(&self) -> Option<Commit> {
        self.current_checkpoint.map(|latest| self.commits[latest].clone())
    }

    /// Given a path to the project.checkpoints file, deserialize the Repo
    pub fn load_project(sentinel: &SentinelFile) -> Result<Repo, err::Reason> {
        let repo_path = sentinel.data_folder().data_json();
        let json = std::fs::read_to_string(&repo_path);

        let repo: Repo = match json {
            Ok(json) => serde_json::from_str(&json).map_err(|e| err::Reason::Json(repo_path, e))?,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Repo::new()),
            Err(e) => return Err(err::Reason::Io(repo_path, e)),
        };

        // Validate before handing the repo to anyone. On any inconsistency:
        // refuse without modifying anything. data.json stays as-is on disk.
        if let Some(c) = repo.current_checkpoint
            && c >= repo.commits.len() {
                return Err(err::Reason::Other(
                    "Project data is inconsistent (checkpoint pointer out of range). \
                     Refusing to open; data.json has not been modified."
                        .to_string(),
                ));
            }
        for commit in &repo.commits {
            for (path, hash) in &commit.files {
                if !repo.file_data.contains_key(hash) {
                    return Err(err::Reason::Other(format!(
                        "Project data is inconsistent: checkpoint '{}' references missing data for {}. \
                         Refusing to open; data.json has not been modified.",
                        commit.message,
                        path.display()
                    )));
                }
            }
        }

        Ok(repo)
    }

    pub fn create_commit(
        &mut self,
        root: &ProjectRoot,
        name: String,
        desc: String,
    ) -> Result<CommitResult, err::Reason> {
        if !self.commits.is_empty() && self.current_checkpoint != Some(self.commits.len() - 1) {
            return Err(err::Reason::Other(
                "Attempted to create commit when on old checkpoint.".to_string(),
            ));
        }

        let name = {
            if name.is_empty() {
                format!("Checkpoint #{}", self.commits.len() + 1)
            } else {
                name
            }
        };

        let delta = self.get_workspace_delta(root)?;
        if delta.is_noop() {
            return Ok(CommitResult::NoOp);
        }

        let mut additions = 0;
        let mut deletions = 0;

        for file in &delta.changed_files {
            let (add, del) = self.get_additions_deletions(&file.0, root);
            additions += add;
            deletions += del;
        }
        for file in &delta.new_files {
            let (add, _) = self.get_additions_deletions(&file.0, root);
            additions += add;
        }
        for file in &delta.deleted_files {
            let (_, del) = self.get_additions_deletions(file, root);
            deletions += del;
        }

        let commit_files = self.snapshot_project(delta)?;

        let prev_checkpoint = self.current_checkpoint;

        self.commits
            .push(Commit::new(name, desc, commit_files, additions, deletions));
        self.current_checkpoint = Some(self.commits.len() - 1);

        if let Err(e) = self.save(root) {
            // Remove the commit from the in-memory representation if it failed to save to disk.
            self.commits.pop();
            self.current_checkpoint = prev_checkpoint;
            return Err(e);
        }

        Ok(CommitResult::Success)
    }

    /// Reverts the project folder to the state of a particular checkpoint
    pub fn restore_checkpoint(
        &mut self,
        commit: usize,
        root: &ProjectRoot,
    ) -> Result<(), err::Reason> {
        if self.commits.len() <= commit {
            return Err(err::Reason::Other(
                "Attempting to restore to a checkpoint that does not exist.".to_string(),
            ));
        }
        self.test_files_in_project_for_locks(root)?;

        let project_path = root.get();
        let checkpoint_data = dunce::canonicalize(root.data_folder().get())
            .map_err(|e| err::Reason::Io(root.get(), e))?;
        let commit_data = &self.commits[commit];

        // Create WAL
        let wal_path = root.data_folder().get().join("restore.wal");
        std::fs::write(wal_path.clone(), format!("{}", commit))
            .map_err(|e| err::Reason::Io(wal_path.clone(), e))?;

        // Delete all the contents
        for entry in std::fs::read_dir(project_path.clone())
            .map_err(|e| err::Reason::Io(project_path.clone(), e))?
        {
            let entry = entry.map_err(|e| {
                err::Reason::IoString("Failed to iterate files during delete step".to_string(), e)
            })?;
            let path = entry.path();
            if dunce::canonicalize(path.clone()).map_err(|e| err::Reason::Io(path.clone(), e))?
                == checkpoint_data
            {
                continue;
            }

            if !root.is_trackable(&path) {
                continue;
            }

            if path.is_dir() {
                std::fs::remove_dir_all(&path).map_err(|e| err::Reason::Io(path, e))?;
            } else {
                std::fs::remove_file(&path).map_err(|e| err::Reason::Io(path, e))?;
            }
        }

        // Recreate files from checkpoint
        for (relative_path, hash) in &commit_data.files {
            let data = &self.file_data[hash];
            let target = project_path.join(relative_path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| err::Reason::Io(parent.to_path_buf(), e))?;
            }
            std::fs::write(&target, &data.0).map_err(|e| err::Reason::Io(target, e))?;
        }

        self.current_checkpoint = Some(commit);
        self.save(root)?;

        // Delete WAL
        std::fs::remove_file(wal_path.clone())
            .map_err(|e| err::Reason::Io(wal_path.clone(), e))?;

        Ok(())
    }

    /// Iterates through the project files to ensure no errors prevent them from being opened.
    /// Returns the path of the first file that is locked, or Ok(()) otherwise.
    pub fn test_files_in_project_for_locks(
        &mut self,
        root: &ProjectRoot,
    ) -> Result<(), err::Reason> {
        let project_path = root.get();

        for entry in walkdir::WalkDir::new(&project_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();

            // Skip your own internal database folder
            if !root.is_trackable(path) {
                continue;
            }

            // If another program has an exclusive write-lock on it (eg it is open in an IDE),
            // or if it's marked read-only by chmod, the OS will return an Err.
            if let Err(err) = OpenOptions::new().write(true).open(path) {
                return Err(err::Reason::Io(path.to_path_buf(), err));
            }
        }

        Ok(())
    }

    /// Atomically update the save data
    pub fn save(&self, root: &ProjectRoot) -> Result<(), err::Reason> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| err::Reason::JsonString("Failed to save. Reason:".to_string(), e))?;
        let tmp_path = root.data_folder().data_tmp();
        let final_path = root.data_folder().data_json();
        std::fs::write(tmp_path.clone(), json)
            .map_err(|e| err::Reason::Io(tmp_path.clone(), e))?;
        std::fs::rename(&tmp_path, &final_path)
            .map_err(|e| err::Reason::Io(tmp_path.clone(), e))?;
        Ok(())
    }

    /// Iterate over the project file system and store the Hash->Data correspondence in the repo. Return a vector of path/hash tuples (which will be used to construct a `Commit`)
    pub fn snapshot_project(
        &mut self,
        delta: WorkspaceDelta,
    ) -> Result<Vec<(PathBuf, FileHash)>, err::Reason> {
        let mut files = {
            if let Some(f) = self.last_commit() {
                f.files
            } else {
                vec![]
            }
        };

        files.retain(|f| delta.deleted_files.iter().find(|x| **x == f.0).is_none());

        for file in delta.changed_files {
            let hash = file.1;
            if let Some(new_data) = file.2 {
                self.file_data.insert(hash, new_data);
            }

            files
                .iter_mut()
                .find(|f| f.0 == file.0)
                .expect("modified file not found in previous commit")
                .1 = hash;
        }

        for file in delta.new_files {
            let hash = file.1;
            if let Some(new_data) = file.2 {
                self.file_data.insert(hash, new_data);
            }

            files.push((file.0, file.1))
        }

        Ok(files)
    }

    /// Given the state of the ProjectRoot directory and the repo, create a  WorkspaceDelta struct that
    /// tracks the modifications, additions, and deletions to the directory.
    pub fn get_workspace_delta(&self, root: &ProjectRoot) -> Result<WorkspaceDelta, err::Reason> {
        let mut changed_files = vec![];
        let mut new_files = vec![];

        let current_commit_data = {
            if let Some(current) = self.current_checkpoint {
                self.commits[current].clone().files
            } else {
                vec![]
            }
        };

        let project_path =
            dunce::canonicalize(root.get()).map_err(|e| err::Reason::Io(root.get(), e))?;

        for entry in walkdir::WalkDir::new(&project_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = dunce::canonicalize(entry.path())
                .map_err(|e| err::Reason::Io(entry.path().into(), e))?;
            if !root.is_trackable(&path) {
                continue;
            }

            let data = std::fs::read(path.clone()).map_err(|e| err::Reason::Io(path.clone(), e))?;
            let hash: FileHash = Sha256::digest(&data).into();

            let relative = path
                .strip_prefix(&project_path)
                .expect("found file outside the project dir")
                .to_path_buf();

            if current_commit_data.contains(&(relative.clone(), hash)) {
                continue;
            }

            // Store the data, or None if this data is already known to the Repo.
            let data_for_this_hash = self.file_data.get(&hash);
            let data = {
                if let Some(_previously_known_data) = data_for_this_hash {
                    None
                } else {
                    Some(FileData(data))
                }
            };

            let relative = path
                .strip_prefix(&project_path)
                .expect("something impossible happened")
                .to_path_buf();

            if current_commit_data
                .iter()
                .find(|data| data.0 == relative)
                .is_some()
            {
                changed_files.push((relative, hash, data));
            } else {
                new_files.push((relative, hash, data));
            }
        }

        let mut deleted_files = vec![];
        for file in current_commit_data {
            let path = file.0;
            let full_path = project_path.join(&path);

            if !full_path.exists() {
                deleted_files.push(path);
            }
        }

        Ok(WorkspaceDelta {
            changed_files,
            deleted_files,
            new_files,
        })
    }

    pub fn get_additions_deletions(
        &self,
        file: &PathBuf,
        project_path: &ProjectRoot,
    ) -> (usize, usize) {
        let full_path = project_path.get().join(file);
        let fallback = FileData(Vec::new());

        let new_bytes = std::fs::read(&full_path).unwrap_or_default();
        let old_bytes = self
            .last_commit()
            .and_then(|commit| {
                commit
                    .files
                    .iter()
                    .find(|(p, _)| p == file)
                    .map(|(_, hash)| *hash)
            })
            .and_then(|hash| self.file_data.get(&hash))
            .unwrap_or(&fallback);

        let old_type = classify_file(&old_bytes.0);
        let new_type = classify_file(&new_bytes);

        let old = match old_type {
            FileType::Utf8Like => String::from_utf8_lossy(&old_bytes.0),
            FileType::Utf16 { little_endian } => decode_utf16(&old_bytes.0, little_endian).into(),
            FileType::Binary => return (0, 0),
        }
        .replace("\r\n", "\n");

        let new = match new_type {
            FileType::Utf8Like => String::from_utf8_lossy(&new_bytes),
            FileType::Utf16 { little_endian } => {
                utilities::decode_utf16(&new_bytes, little_endian).into()
            }
            FileType::Binary => return (0, 0),
        }
        .replace("\r\n", "\n");

        let diff = similar::TextDiff::from_lines(old.as_str(), new.as_str());
        let mut additions = 0;
        let mut deletions = 0;
        for change in diff.iter_all_changes() {
            match change.tag() {
                similar::ChangeTag::Insert => additions += 1,
                similar::ChangeTag::Delete => deletions += 1,
                similar::ChangeTag::Equal => {}
            }
        }
        (additions, deletions)
    }

    /// Returns true if the current commit is the last commit.
    pub fn is_on_latest_checkpoint(&self) -> bool {
        match self.current_checkpoint {
            Some(commit) => commit + 1 == self.commits.len(),
            None => self.commits.is_empty(),
        }
    }

    /// Number of additions per minute compared to previous commit.
    pub fn additions_per_minute(&self, commit_index: usize) -> Option<f64> {
        if commit_index == 0 {
            return None;
        }
        let cur = &self.commits[commit_index];
        let prev = &self.commits[commit_index - 1];
        let secs = (cur.timestamp - prev.timestamp).num_seconds();
        if secs <= 0 {
            return None;
        }
        Some(cur.additions as f64 / (secs as f64 / 60.0))
    }
}

pub struct WorkspaceDelta {
    pub new_files: Vec<(PathBuf, FileHash, Option<FileData>)>,
    pub changed_files: Vec<(PathBuf, FileHash, Option<FileData>)>,
    pub deleted_files: Vec<PathBuf>,
}

impl WorkspaceDelta {
    pub fn is_noop(&self) -> bool {
        self.new_files.is_empty() && self.changed_files.is_empty() && self.deleted_files.is_empty()
    }
}

pub struct App {
    pub repo: Repo,
    pub selected: Option<usize>,
    pub registration: Option<RegistrationInfo>,
    pub registration_status: Option<Registration>,

    pub ui_data: UIData,
}

impl App {
    pub fn new(license: Option<RegistrationInfo>) -> Self {
        Self {
            repo: Repo::new(),
            selected: None,
            registration: license.clone(),
            registration_status: license.map(|x| x.validate()),
            ui_data: UIData::default(),
        }
    }
}
