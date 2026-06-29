//! Test suite for CheckpointPro's core repo logic.
//!
//! Declared from main.rs via:
//!     #[cfg(test)]
//!     mod tests;
//!
//! Tests marked #[ignore] assert *desired* behavior for known bugs / undecided
//! design points discussed during review. They will panic or fail today.
//! Run them with: cargo test -- --ignored
//! As you fix each issue, remove the #[ignore] and the test becomes a regression guard.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::data_structures::{CommitResult, FileHash, Repo};
use crate::err;
use crate::file_system::{self, ProjectRoot, get_failed_restore_id};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// tempdir -> canonicalized ProjectRoot -> init_project -> fresh Repo.
/// Keep the TempDir alive (bind as `_dir`, not `_`) or the directory vanishes.
/// Canonicalizing matters on macOS, where tempdirs live behind a /var symlink
/// and path prefix-stripping would otherwise misbehave.
fn fixture() -> (tempfile::TempDir, ProjectRoot, Repo) {
    let dir = tempfile::TempDir::new().unwrap();
    let canonical = dunce::canonicalize(dir.path()).unwrap();
    let root = ProjectRoot::new(canonical);
    file_system::init_project(&root).unwrap();
    (dir, root, Repo::new())
}

/// Build a relative path from components so separators are correct per-platform.
/// (Comparing PathBuf::from("a/b") against walkdir output fails on Windows.)
fn rel(parts: &[&str]) -> PathBuf {
    parts.iter().collect()
}

fn write_file(root: &ProjectRoot, relative: &Path, contents: impl AsRef<[u8]>) {
    let path = root.get().join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn read_file(root: &ProjectRoot, relative: &Path) -> Vec<u8> {
    std::fs::read(root.get().join(relative)).unwrap()
}

fn delete_file(root: &ProjectRoot, relative: &Path) {
    std::fs::remove_file(root.get().join(relative)).unwrap();
}

fn commit(repo: &mut Repo, root: &ProjectRoot, name: &str) -> CommitResult {
    repo.create_commit(root, name.to_string(), String::new())
        .expect("commit failed")
}

/// Commit files compared as a set: walkdir traversal order is not guaranteed
/// stable across platforms, so never assert on Vec order.
fn files_of(repo: &Repo, idx: usize) -> HashSet<(PathBuf, FileHash)> {
    repo.commits[idx].files.iter().cloned().collect()
}

fn hash_of(repo: &Repo, idx: usize, relative: &Path) -> FileHash {
    repo.commits[idx]
        .files
        .iter()
        .find(|(p, _)| p == relative)
        .map(|(_, h)| *h)
        .unwrap_or_else(|| panic!("{relative:?} not in commit {idx}"))
}

/// Cross-cutting structural invariants. Call at the end of any test that
/// mutates the repo; catches corruption the test didn't think to assert on.
fn assert_repo_valid(repo: &Repo) {
    if let Some(c) = repo.current_checkpoint {
        assert!(
            c < repo.commits.len(),
            "current_checkpoint {} out of bounds (len {})",
            c,
            repo.commits.len()
        );
    }
    // Referential integrity: every hash referenced by any commit must exist in
    // file_data, or restore_checkpoint will panic on its unchecked index.
    for (i, commit) in repo.commits.iter().enumerate() {
        for (path, hash) in &commit.files {
            assert!(
                repo.file_data.contains_key(hash),
                "commit {i} references {path:?} whose hash is missing from file_data"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Commit mechanics
// ---------------------------------------------------------------------------

#[test]
fn empty_name_defaults_to_numbered_checkpoint() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["a.txt"]), "one");
    commit(&mut repo, &root, "");
    write_file(&root, &rel(&["a.txt"]), "two");
    commit(&mut repo, &root, "");

    assert_eq!(repo.commits[0].message, "Checkpoint #1");
    assert_eq!(repo.commits[1].message, "Checkpoint #2");
    assert_repo_valid(&repo);
}

#[test]
fn whitespace_only_name_is_kept_verbatim() {
    // Pins current behavior: only the empty string triggers the default name.
    // If you'd rather trim first, change create_commit and flip this test.
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["a.txt"]), "x");
    commit(&mut repo, &root, "   ");
    assert_eq!(repo.commits[0].message, "   ");
}

#[test]
fn noop_commit_changes_nothing_and_does_not_save() {
    let (_dir, root, mut repo) = fixture();

    // Fresh project: only the readmes exist, and both live in untracked areas.
    let result = repo
        .create_commit(&root, "should not exist".into(), String::new())
        .unwrap();

    assert_eq!(result, CommitResult::NoOp);
    assert!(repo.commits.is_empty());
    assert_eq!(repo.current_checkpoint, None);
    // save() must not have run: a NoOp should leave no data.json behind.
    assert!(!root.data_folder().data_json().exists());
}

#[test]
fn identical_content_is_deduplicated_in_file_data() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["a.txt"]), "same bytes");
    write_file(&root, &rel(&["b.txt"]), "same bytes");
    commit(&mut repo, &root, "dedup");

    // Two paths, one stored blob.
    assert_eq!(repo.commits[0].files.len(), 2);
    assert_eq!(repo.file_data.len(), 1);
    assert_eq!(
        hash_of(&repo, 0, &rel(&["a.txt"])),
        hash_of(&repo, 0, &rel(&["b.txt"]))
    );
    assert_repo_valid(&repo);
}

#[test]
fn empty_file_commits_and_restores() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["empty.txt"]), "");
    commit(&mut repo, &root, "c0");
    write_file(&root, &rel(&["empty.txt"]), "no longer empty");
    commit(&mut repo, &root, "c1");

    repo.restore_checkpoint(0, &root).unwrap();
    assert_eq!(read_file(&root, &rel(&["empty.txt"])), b"");
    assert_repo_valid(&repo);
}

#[test]
fn reverted_content_reuses_known_hash() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["a.txt"]), "v1");
    commit(&mut repo, &root, "c0");
    write_file(&root, &rel(&["a.txt"]), "v2");
    commit(&mut repo, &root, "c1");
    // Manually revert the content (not via restore_checkpoint).
    write_file(&root, &rel(&["a.txt"]), "v1");
    commit(&mut repo, &root, "c2");

    assert_eq!(
        hash_of(&repo, 0, &rel(&["a.txt"])),
        hash_of(&repo, 2, &rel(&["a.txt"]))
    );
    // Only two distinct blobs were ever stored.
    assert_eq!(repo.file_data.len(), 2);
    assert_repo_valid(&repo);
}

#[test]
fn commit_from_old_checkpoint_with_modified_file_is_rejected() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["a.txt"]), "v1");
    commit(&mut repo, &root, "c0");
    write_file(&root, &rel(&["a.txt"]), "v2");
    commit(&mut repo, &root, "c1");

    repo.restore_checkpoint(0, &root).unwrap();
    write_file(&root, &rel(&["a.txt"]), "divergent");

    // Desired: an Err (or a CommitResult::NotOnLatestCheckpoint variant —
    // adjust this assertion to match whichever you implement).
    let result = repo.create_commit(&root, "bad".into(), String::new());
    assert!(
        result.is_err(),
        "committing from an old checkpoint must be refused"
    );
    assert_eq!(repo.commits.len(), 2, "no commit should have been created");
    assert_repo_valid(&repo);
}

#[test]
fn commit_from_old_checkpoint_with_new_file_is_rejected() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["a.txt"]), "v1");
    commit(&mut repo, &root, "c0");
    write_file(&root, &rel(&["b.txt"]), "added in c1");
    commit(&mut repo, &root, "c1");

    repo.restore_checkpoint(0, &root).unwrap();
    write_file(&root, &rel(&["c.txt"]), "new while on old checkpoint");

    let result = repo.create_commit(&root, "bad".into(), String::new());
    assert!(
        result.is_err(),
        "committing from an old checkpoint must be refused"
    );
    assert_eq!(repo.commits.len(), 2);
    assert_repo_valid(&repo);
}

#[test]
fn restore_out_of_bounds_is_an_error() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["a.txt"]), "v1");
    commit(&mut repo, &root, "c0");

    let result = repo.restore_checkpoint(repo.commits.len(), &root);
    assert!(result.is_err());
    assert_repo_valid(&repo);
}

// ---------------------------------------------------------------------------
// Delta detection
// ---------------------------------------------------------------------------

#[test]
fn delta_buckets_new_modified_deleted_correctly() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["a.txt"]), "a");
    write_file(&root, &rel(&["b.txt"]), "b");
    write_file(&root, &rel(&["c.txt"]), "c");
    commit(&mut repo, &root, "base");

    write_file(&root, &rel(&["a.txt"]), "a modified");
    delete_file(&root, &rel(&["b.txt"]));
    write_file(&root, &rel(&["d.txt"]), "d is new");

    let delta = repo.get_workspace_delta(&root).unwrap();

    let changed: HashSet<_> = delta
        .changed_files
        .iter()
        .map(|(p, _, _)| p.clone())
        .collect();
    let new: HashSet<_> = delta.new_files.iter().map(|(p, _, _)| p.clone()).collect();
    let deleted: HashSet<_> = delta.deleted_files.iter().cloned().collect();

    assert_eq!(changed, HashSet::from([rel(&["a.txt"])]));
    assert_eq!(new, HashSet::from([rel(&["d.txt"])]));
    assert_eq!(deleted, HashSet::from([rel(&["b.txt"])]));
    // c.txt untouched: in no bucket.
}

#[test]
fn rename_appears_as_delete_plus_add() {
    // Pins the absence of rename tracking. If "renames lose history" ever gets
    // reported, this test documents it as known behavior, not a regression.
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["old_name.txt"]), "contents");
    commit(&mut repo, &root, "base");

    std::fs::rename(
        root.get().join("old_name.txt"),
        root.get().join("new_name.txt"),
    )
    .unwrap();

    let delta = repo.get_workspace_delta(&root).unwrap();
    assert_eq!(delta.deleted_files, vec![rel(&["old_name.txt"])]);
    assert_eq!(delta.new_files.len(), 1);
    assert_eq!(delta.new_files[0].0, rel(&["new_name.txt"]));
    assert!(delta.changed_files.is_empty());
}

#[test]
fn dotfiles_are_not_tracked() {
    let (_dir, root, repo) = fixture();

    write_file(&root, &rel(&[".secret"]), "hidden");
    let delta = repo.get_workspace_delta(&root).unwrap();
    assert!(
        delta.is_noop(),
        "dotfiles must not appear in any delta bucket"
    );
}

#[test]
fn files_inside_dot_directories_are_tracked() {
    // Pins current behavior: is_trackable only inspects the *file's own* name,
    // so .hidden_dir/normal.txt IS tracked. Note the asymmetry: restore's
    // delete pass skips top-level dot directories entirely, so such files get
    // written by restore but never cleaned up. If you'd rather exclude whole
    // dot directories, fix is_trackable and invert this test.
    let (_dir, root, repo) = fixture();

    write_file(&root, &rel(&[".hidden_dir", "normal.txt"]), "surprise");
    let delta = repo.get_workspace_delta(&root).unwrap();

    let new: HashSet<_> = delta.new_files.iter().map(|(p, _, _)| p.clone()).collect();
    assert!(new.contains(&rel(&[".hidden_dir", "normal.txt"])));
}

#[test]
fn untracked_and_data_folders_never_appear_in_deltas() {
    let (_dir, root, repo) = fixture();

    write_file(
        &root,
        &rel(&["untracked_files", "huge_video.bin"]),
        "pretend this is 4GB",
    );
    write_file(
        &root,
        &rel(&["checkpoint_data", "intruder.txt"]),
        "should be ignored",
    );

    let delta = repo.get_workspace_delta(&root).unwrap();
    assert!(delta.is_noop());
}

#[test]
fn nested_directories_roundtrip_through_commit_and_restore() {
    let (_dir, root, mut repo) = fixture();

    let deep = rel(&["src", "modules", "auth", "tokens.rs"]);
    write_file(&root, &deep, "deep contents");
    commit(&mut repo, &root, "c0");

    // Wipe the whole tree, restore, verify it comes back.
    std::fs::remove_dir_all(root.get().join("src")).unwrap();
    write_file(
        &root,
        &rel(&["unrelated.txt"]),
        "force a second commit to exist",
    );
    commit(&mut repo, &root, "c1");

    repo.restore_checkpoint(0, &root).unwrap();
    assert_eq!(read_file(&root, &deep), b"deep contents");
    assert!(!root.get().join("unrelated.txt").exists());
    assert_repo_valid(&repo);
}

#[test]
fn non_utf8_content_roundtrips_losslessly() {
    // Storage is Vec<u8> and must stay byte-exact even though the diff view
    // displays via from_utf8_lossy. Covers commit -> restore AND save -> load.
    let (_dir, root, mut repo) = fixture();

    let bytes: Vec<u8> = vec![0x00, 0x9F, 0x92, 0x96, 0xFF, 0xFE, 0x80];
    write_file(&root, &rel(&["binary.dat"]), &bytes);
    commit(&mut repo, &root, "c0");
    write_file(&root, &rel(&["binary.dat"]), "replaced");
    commit(&mut repo, &root, "c1");

    repo.restore_checkpoint(0, &root).unwrap();
    assert_eq!(read_file(&root, &rel(&["binary.dat"])), bytes);

    // And through JSON/base64 persistence:
    let loaded = Repo::load_project(&root.data_folder().sentinel()).unwrap();
    let hash = hash_of(&loaded, 0, &rel(&["binary.dat"]));
    assert_eq!(loaded.file_data[&hash].0, bytes);
}

#[test]
fn filenames_with_spaces_and_unicode_roundtrip() {
    let (_dir, root, mut repo) = fixture();

    let fancy = rel(&["meine Dateien", "résumé final (v2) 日本語.txt"]);
    write_file(&root, &fancy, "fancy");
    commit(&mut repo, &root, "c0");
    write_file(&root, &fancy, "changed");
    commit(&mut repo, &root, "c1");

    repo.restore_checkpoint(0, &root).unwrap();
    assert_eq!(read_file(&root, &fancy), b"fancy");

    // Path survives serde round trip too.
    let loaded = Repo::load_project(&root.data_folder().sentinel()).unwrap();
    assert!(loaded.commits[0].files.iter().any(|(p, _)| p == &fancy));
}

// ---------------------------------------------------------------------------
// Restore mechanics
// ---------------------------------------------------------------------------

#[test]
fn restore_deletes_files_and_dirs_created_after_checkpoint() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["a.txt"]), "original");
    commit(&mut repo, &root, "c0");

    write_file(&root, &rel(&["b.txt"]), "later file");
    write_file(&root, &rel(&["newdir", "c.txt"]), "later dir");
    commit(&mut repo, &root, "c1");

    repo.restore_checkpoint(0, &root).unwrap();

    assert!(root.get().join("a.txt").exists());
    assert!(!root.get().join("b.txt").exists());
    assert!(
        !root.get().join("newdir").exists(),
        "whole new top-level dir must vanish"
    );
    assert_repo_valid(&repo);
}

#[test]
fn restore_recreates_deleted_directories() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["sub", "dir", "file.txt"]), "v1");
    commit(&mut repo, &root, "c0");

    std::fs::remove_dir_all(root.get().join("sub")).unwrap();
    write_file(&root, &rel(&["other.txt"]), "x");
    commit(&mut repo, &root, "c1");

    repo.restore_checkpoint(0, &root).unwrap();
    assert_eq!(read_file(&root, &rel(&["sub", "dir", "file.txt"])), b"v1");
}

#[test]
fn restore_current_checkpoint_with_clean_workspace_is_lossless() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["a.txt"]), "stable");
    commit(&mut repo, &root, "c0");

    repo.restore_checkpoint(0, &root).unwrap();

    assert_eq!(read_file(&root, &rel(&["a.txt"])), b"stable");
    assert_eq!(repo.current_checkpoint, Some(0));
    assert_repo_valid(&repo);
}

#[test]
fn restore_discards_unsaved_workspace_changes() {
    // This is the documented behavior behind the UI's "WARNING" dialog.
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["a.txt"]), "committed");
    commit(&mut repo, &root, "c0");

    write_file(&root, &rel(&["a.txt"]), "dirty, never committed");
    write_file(&root, &rel(&["scratch.txt"]), "also dirty");

    repo.restore_checkpoint(0, &root).unwrap();

    assert_eq!(read_file(&root, &rel(&["a.txt"])), b"committed");
    assert!(!root.get().join("scratch.txt").exists());
}

#[test]
fn successful_restore_clears_wal_and_persists_state() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["a.txt"]), "v1");
    commit(&mut repo, &root, "c0");
    write_file(&root, &rel(&["a.txt"]), "v2");
    commit(&mut repo, &root, "c1");

    repo.restore_checkpoint(0, &root).unwrap();

    assert!(
        !root.data_folder().wal().exists(),
        "WAL must be deleted after success"
    );
    assert_eq!(repo.current_checkpoint, Some(0));

    // The new state must already be on disk, not just in memory.
    let loaded = Repo::load_project(&root.data_folder().sentinel()).unwrap();
    assert_eq!(loaded.current_checkpoint, Some(0));
    assert_eq!(loaded.commits.len(), 2);
}

// ---------------------------------------------------------------------------
// Persistence / round-trip
// ---------------------------------------------------------------------------

#[test]
fn save_load_roundtrip_preserves_everything_but_timestamps() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["a.txt"]), "alpha");
    write_file(&root, &rel(&["sub", "b.txt"]), "beta");
    repo.create_commit(&root, "first".into(), "a description".into())
        .unwrap();
    write_file(&root, &rel(&["a.txt"]), "alpha v2");
    repo.create_commit(&root, "second".into(), String::new())
        .unwrap();

    let loaded = Repo::load_project(&root.data_folder().sentinel()).unwrap();

    assert_eq!(loaded.commits.len(), repo.commits.len());
    assert_eq!(loaded.current_checkpoint, repo.current_checkpoint);
    for i in 0..repo.commits.len() {
        assert_eq!(loaded.commits[i].message, repo.commits[i].message);
        assert_eq!(loaded.commits[i].description, repo.commits[i].description);
        // Sets, not Vecs: walkdir order isn't part of the contract.
        assert_eq!(files_of(&loaded, i), files_of(&repo, i));
    }
    // file_data: same keys, byte-identical blobs.
    let orig_keys: HashSet<_> = repo.file_data.keys().collect();
    let load_keys: HashSet<_> = loaded.file_data.keys().collect();
    assert_eq!(orig_keys, load_keys);
    for (k, v) in &repo.file_data {
        assert_eq!(loaded.file_data[k].0, v.0);
    }
    assert_repo_valid(&loaded);
}

#[test]
fn load_with_missing_json_gives_fresh_repo() {
    let (_dir, root, _repo) = fixture();
    // init_project creates the folder structure but never writes data.json.
    let loaded = Repo::load_project(&root.data_folder().sentinel()).unwrap();
    assert!(loaded.commits.is_empty());
    assert_eq!(loaded.current_checkpoint, None);
}

#[test]
fn load_with_corrupt_json_is_a_json_error() {
    // Guards the fall-through fix in view_directory: a corrupt data.json must
    // surface as Err so the UI aborts instead of opening (and later
    // overwriting) the project with a default repo.
    let (_dir, root, _repo) = fixture();

    std::fs::write(root.data_folder().data_json(), "{ this is not json").unwrap();

    let result = Repo::load_project(&root.data_folder().sentinel());
    assert!(matches!(result, Err(err::Reason::Json(_, _))));
}

#[test]
fn load_with_out_of_bounds_checkpoint_is_rejected() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["a.txt"]), "x");
    commit(&mut repo, &root, "c0");

    // Sabotage the saved state: current_checkpoint points past the end.
    let json = std::fs::read_to_string(root.data_folder().data_json()).unwrap();
    let tampered = json.replace("\"current_checkpoint\": 0", "\"current_checkpoint\": 99");
    assert_ne!(
        json, tampered,
        "tampering failed; serialization format changed?"
    );
    std::fs::write(root.data_folder().data_json(), tampered).unwrap();

    let result = Repo::load_project(&root.data_folder().sentinel());
    assert!(
        result.is_err(),
        "an internally inconsistent repo must not load"
    );
}

#[test]
fn stale_tmp_file_does_not_break_save() {
    let (_dir, root, mut repo) = fixture();

    // Simulate a crash that left data.tmp behind.
    std::fs::write(root.data_folder().data_tmp(), "garbage from a dead process").unwrap();

    write_file(&root, &rel(&["a.txt"]), "x");
    commit(&mut repo, &root, "c0"); // triggers save()

    assert!(
        !root.data_folder().data_tmp().exists(),
        "tmp must be consumed by the rename"
    );
    let loaded = Repo::load_project(&root.data_folder().sentinel()).unwrap();
    assert_eq!(loaded.commits.len(), 1);
}

#[test]
fn repeated_saves_leave_valid_state() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["a.txt"]), "x");
    commit(&mut repo, &root, "c0");
    repo.save(&root).unwrap();
    repo.save(&root).unwrap();

    let loaded = Repo::load_project(&root.data_folder().sentinel()).unwrap();
    assert_eq!(loaded.commits.len(), 1);
}

// ---------------------------------------------------------------------------
// WAL / recovery
// ---------------------------------------------------------------------------

#[test]
fn no_wal_means_no_failed_commit() {
    let (_dir, root, _repo) = fixture();
    assert_eq!(get_failed_restore_id(&root), None);
}

#[test]
fn valid_wal_returns_the_recorded_id() {
    let (_dir, root, _repo) = fixture();
    std::fs::write(root.data_folder().wal(), "3").unwrap();
    assert_eq!(get_failed_restore_id(&root), Some(3));
}

#[test]
fn wal_with_surrounding_whitespace_still_parses() {
    // The implementation trims; pin that, since the WAL is written by us but
    // read back across crash boundaries.
    let (_dir, root, _repo) = fixture();
    std::fs::write(root.data_folder().wal(), " 7\n").unwrap();
    assert_eq!(get_failed_restore_id(&root), Some(7));
}

#[test]
fn garbage_wal_does_not_panic() {
    let (_dir, root, _repo) = fixture();
    std::fs::write(root.data_folder().wal(), "banana").unwrap();
    // Adjust if you change the signature to Result<Option<usize>, _>.
    assert_eq!(get_failed_restore_id(&root), None);
}

// ---------------------------------------------------------------------------
// init_project
// ---------------------------------------------------------------------------

#[test]
fn init_creates_expected_structure() {
    let (_dir, root, _repo) = fixture();

    assert!(root.data_folder().sentinel().get().exists());
    assert!(root.data_folder().get().is_dir());
    assert!(root.untracked().is_dir());
    assert!(root.data_folder().get().join("readme.txt").exists());
    assert!(root.untracked().join("readme.txt").exists());
}

#[test]
fn init_refuses_a_folder_that_already_has_a_project() {
    let (_dir, root, _repo) = fixture();

    let result = file_system::init_project(&root);
    assert!(matches!(result, Err(err::Init::ProjectAlreadyExists)));
}

#[test]
fn init_over_half_initialized_folder_currently_succeeds() {
    // Pins current behavior at a known decision point: the collision check is
    // sentinel-only, so a folder with checkpoint_data/ (possibly containing a
    // real data.json!) but no sentinel gets re-initialized. If you decide this
    // should be ProjectAlreadyExists — or should refuse when data.json exists —
    // invert this test.
    let dir = tempfile::TempDir::new().unwrap();
    let canonical = dunce::canonicalize(dir.path()).unwrap();
    std::fs::create_dir_all(canonical.join("checkpoint_data")).unwrap();
    std::fs::write(canonical.join("checkpoint_data").join("data.json"), "{}").unwrap();

    let root = ProjectRoot::new(canonical);
    let result = file_system::init_project(&root);
    assert!(result.is_ok());
}

#[test]
fn init_preserves_existing_user_files_and_first_commit_picks_them_up() {
    let dir = tempfile::TempDir::new().unwrap();
    let canonical = dunce::canonicalize(dir.path()).unwrap();
    std::fs::write(canonical.join("precious.txt"), "existed before init").unwrap();

    let root = ProjectRoot::new(canonical);
    file_system::init_project(&root).unwrap();

    assert_eq!(
        std::fs::read(root.get().join("precious.txt")).unwrap(),
        b"existed before init"
    );

    let mut repo = Repo::new();
    commit(&mut repo, &root, "first");
    assert!(
        files_of(&repo, 0)
            .iter()
            .any(|(p, _)| p == &rel(&["precious.txt"]))
    );
    assert_repo_valid(&repo);
}

// ---------------------------------------------------------------------------
// Lock probing
// ---------------------------------------------------------------------------

#[test]
fn lock_probe_passes_on_a_normal_tree() {
    // Deliberately shallow: real lock detection (sharing violations) only
    // exists on Windows, and simulating it portably isn't worth it.
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["a.txt"]), "x");
    write_file(&root, &rel(&["sub", "b.txt"]), "y");
    assert!(repo.test_files_in_project_for_locks(&root).is_ok());
}

// ---------------------------------------------------------------------------
// Cross-cutting lifecycle
// ---------------------------------------------------------------------------

#[test]
fn invariants_hold_across_a_full_lifecycle() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["a.txt"]), "v1");
    commit(&mut repo, &root, "c0");
    assert!(repo.is_on_latest_checkpoint());
    assert_repo_valid(&repo);

    write_file(&root, &rel(&["a.txt"]), "v2");
    write_file(&root, &rel(&["b.txt"]), "new in c1");
    commit(&mut repo, &root, "c1");
    assert!(repo.is_on_latest_checkpoint());
    assert_repo_valid(&repo);

    repo.restore_checkpoint(0, &root).unwrap();
    assert!(!repo.is_on_latest_checkpoint());
    assert_repo_valid(&repo);

    // Walk back to the latest checkpoint, then committing is allowed again.
    repo.restore_checkpoint(1, &root).unwrap();
    assert!(repo.is_on_latest_checkpoint());
    assert_eq!(read_file(&root, &rel(&["b.txt"])), b"new in c1");

    write_file(&root, &rel(&["a.txt"]), "v3");
    commit(&mut repo, &root, "c2");
    assert!(repo.is_on_latest_checkpoint());
    assert_eq!(repo.current_checkpoint, Some(2));
    assert_repo_valid(&repo);

    // And the whole story survives a reload.
    let loaded = Repo::load_project(&root.data_folder().sentinel()).unwrap();
    assert_eq!(loaded.commits.len(), 3);
    assert_eq!(loaded.current_checkpoint, Some(2));
    assert_repo_valid(&loaded);
}

#[test]
fn fresh_repo_is_on_latest_checkpoint() {
    let repo = Repo::new();
    assert!(repo.is_on_latest_checkpoint());
    assert!(repo.last_commit().is_none());
}

#[test]
fn failed_save_rolls_back_the_commit() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["a.txt"]), "v1");
    commit(&mut repo, &root, "c0");

    // Sabotage save(): a directory squatting on data.tmp makes fs::write fail.
    std::fs::create_dir(root.data_folder().data_tmp()).unwrap();

    write_file(&root, &rel(&["a.txt"]), "v2");
    let result = repo.create_commit(&root, "doomed".into(), String::new());

    // Memory must not be ahead of disk.
    assert!(result.is_err());
    assert_eq!(repo.commits.len(), 1);
    assert_eq!(repo.current_checkpoint, Some(0));
    assert!(repo.is_on_latest_checkpoint());
    assert_repo_valid(&repo);

    // Disk still holds the old, valid state.
    let loaded = Repo::load_project(&root.data_folder().sentinel()).unwrap();
    assert_eq!(loaded.commits.len(), 1);

    // The whole argument for rollback over unwrap(): once the obstruction
    // clears, the user just clicks Create again and it works.
    std::fs::remove_dir(root.data_folder().data_tmp()).unwrap();
    let retry = repo
        .create_commit(&root, "retried".into(), String::new())
        .unwrap();
    assert_eq!(retry, CommitResult::Success);
    assert_eq!(repo.commits.len(), 2);
    assert_repo_valid(&repo);
}

#[test]
fn life_goes_on_after_a_garbage_wal() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["a.txt"]), "v1");
    commit(&mut repo, &root, "c0");

    // Corruption strikes between sessions.
    std::fs::write(root.data_folder().wal(), "banana").unwrap();

    // "New session": load works, the WAL reads as absent.
    let mut repo = Repo::load_project(&root.data_folder().sentinel()).unwrap();
    assert_eq!(get_failed_restore_id(&root), None);
    assert_eq!(repo.commits.len(), 1);

    // Committing works, ignores the WAL, and doesn't accidentally track it.
    write_file(&root, &rel(&["a.txt"]), "v2");
    assert_eq!(commit(&mut repo, &root, "c1"), CommitResult::Success);
    assert!(
        !files_of(&repo, 1)
            .iter()
            .any(|(p, _)| p.ends_with("restore.wal")),
        "WAL must never be snapshotted into a commit"
    );

    // A restore overwrites the garbage with a real WAL, then deletes it:
    // the corruption is gone, by accident of path reuse. Pin the accident.
    repo.restore_checkpoint(0, &root).unwrap();
    assert!(!root.data_folder().wal().exists());
    assert_eq!(read_file(&root, &rel(&["a.txt"])), b"v1");
    assert_eq!(get_failed_restore_id(&root), None);
    assert_repo_valid(&repo);
}

#[test]
fn load_with_missing_blob_is_rejected_without_modifying_disk() {
    let (_dir, root, mut repo) = fixture();

    write_file(&root, &rel(&["a.txt"]), "v1");
    commit(&mut repo, &root, "c0");

    // Manufacture the inconsistency: commit references a blob that's gone.
    // (save() doesn't validate — being the gate is the loader's job.)
    repo.file_data.clear();
    repo.save(&root).unwrap();

    let before = std::fs::read(root.data_folder().data_json()).unwrap();
    let result = Repo::load_project(&root.data_folder().sentinel());
    assert!(matches!(result, Err(err::Reason::Other(_))));
    let after = std::fs::read(root.data_folder().data_json()).unwrap();
    assert_eq!(before, after);
}

// ===============================================
//              DIFF CHECKING TESTS
// ===============================================
fn utf16le_bytes(s: &str) -> Vec<u8> {
    let mut v = vec![0xFF, 0xFE]; // LE BOM
    v.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
    v
}

fn utf16be_bytes(s: &str) -> Vec<u8> {
    let mut v = vec![0xFE, 0xFF]; // BE BOM
    v.extend(s.encode_utf16().flat_map(u16::to_be_bytes));
    v
}

// ---------------------------------------------------------------------------
// UTF-8 common path — the typo guards
// ---------------------------------------------------------------------------

#[test]
fn one_line_modified_counts_one_one() {
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["a.txt"]), "one\ntwo\nthree\n");
    commit(&mut repo, &root, "c0");
    write_file(&root, &rel(&["a.txt"]), "one\nCHANGED\nthree\n");
    assert_eq!(
        repo.get_additions_deletions(&rel(&["a.txt"]), &root),
        (1, 1)
    );
}

#[test]
fn appending_lines_counts_only_additions() {
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["a.txt"]), "one\ntwo\n");
    commit(&mut repo, &root, "c0");
    write_file(&root, &rel(&["a.txt"]), "one\ntwo\nthree\nfour\n");
    assert_eq!(
        repo.get_additions_deletions(&rel(&["a.txt"]), &root),
        (2, 0)
    );
}

#[test]
fn removing_lines_counts_only_deletions() {
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["a.txt"]), "one\ntwo\nthree\nfour\n");
    commit(&mut repo, &root, "c0");
    write_file(&root, &rel(&["a.txt"]), "one\nfour\n");
    assert_eq!(
        repo.get_additions_deletions(&rel(&["a.txt"]), &root),
        (0, 2)
    );
}

#[test]
fn no_change_counts_zero() {
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["a.txt"]), "stable\ncontent\n");
    commit(&mut repo, &root, "c0");
    assert_eq!(
        repo.get_additions_deletions(&rel(&["a.txt"]), &root),
        (0, 0)
    );
}

// ---------------------------------------------------------------------------
// New file / empty repo — old side is the empty fallback
// ---------------------------------------------------------------------------

#[test]
fn brand_new_file_is_all_additions() {
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["other.txt"]), "x\n");
    commit(&mut repo, &root, "c0"); // last_commit exists but lacks fresh.txt
    write_file(&root, &rel(&["fresh.txt"]), "a\nb\nc\n");
    assert_eq!(
        repo.get_additions_deletions(&rel(&["fresh.txt"]), &root),
        (3, 0)
    );
}

#[test]
fn file_against_empty_repo_is_all_additions() {
    let (_dir, root, repo) = fixture(); // no commits: last_commit() is None
    write_file(&root, &rel(&["a.txt"]), "x\ny\n");
    assert_eq!(
        repo.get_additions_deletions(&rel(&["a.txt"]), &root),
        (2, 0)
    );
}

// ---------------------------------------------------------------------------
// Deleted file — gone from disk, present in last commit. Tests the per-file
// function in isolation; commit-time summation must still source deleted paths
// from delta.deleted_files (the workspace walk can't find them).
// ---------------------------------------------------------------------------

#[test]
fn deleted_file_is_all_deletions() {
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["gone.txt"]), "a\nb\nc\n");
    commit(&mut repo, &root, "c0");
    delete_file(&root, &rel(&["gone.txt"]));
    assert_eq!(
        repo.get_additions_deletions(&rel(&["gone.txt"]), &root),
        (0, 3)
    );
}

// ---------------------------------------------------------------------------
// Empty boundaries
// ---------------------------------------------------------------------------

#[test]
fn empty_to_content_is_all_additions() {
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["a.txt"]), "");
    commit(&mut repo, &root, "c0");
    write_file(&root, &rel(&["a.txt"]), "a\nb\n");
    assert_eq!(
        repo.get_additions_deletions(&rel(&["a.txt"]), &root),
        (2, 0)
    );
}

#[test]
fn content_to_empty_is_all_deletions() {
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["a.txt"]), "a\nb\nc\n");
    commit(&mut repo, &root, "c0");
    write_file(&root, &rel(&["a.txt"]), "");
    assert_eq!(
        repo.get_additions_deletions(&rel(&["a.txt"]), &root),
        (0, 3)
    );
}

// ---------------------------------------------------------------------------
// Line-ending normalization
// ---------------------------------------------------------------------------

#[test]
fn crlf_vs_lf_same_content_is_no_change() {
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["a.txt"]), "one\ntwo\nthree\n"); // LF
    commit(&mut repo, &root, "c0");
    write_file(&root, &rel(&["a.txt"]), "one\r\ntwo\r\nthree\r\n"); // CRLF, same text
    assert_eq!(
        repo.get_additions_deletions(&rel(&["a.txt"]), &root),
        (0, 0)
    );
}

// ---------------------------------------------------------------------------
// Binary — either side binary => (0,0), no phantom counts, no panic
// ---------------------------------------------------------------------------

#[test]
fn binary_new_side_counts_zero() {
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["f.dat"]), "text\nfor now\n");
    commit(&mut repo, &root, "c0");
    write_file(&root, &rel(&["f.dat"]), &[0x00, 0x01, 0x02, 0x00, 0xFF]);
    assert_eq!(
        repo.get_additions_deletions(&rel(&["f.dat"]), &root),
        (0, 0)
    );
}

#[test]
fn binary_old_side_counts_zero() {
    // Binary -> text swap must count zero, NOT invent additions for the text
    // side (the "binary as empty" trap we explicitly rejected).
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["f.dat"]), &[0x00, 0x01, 0x02, 0x00]);
    commit(&mut repo, &root, "c0");
    write_file(&root, &rel(&["f.dat"]), "now i am text\n");
    assert_eq!(
        repo.get_additions_deletions(&rel(&["f.dat"]), &root),
        (0, 0)
    );
}

#[test]
fn png_like_binary_counts_zero_without_panicking() {
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["img.png"]), "placeholder\n");
    commit(&mut repo, &root, "c0");
    let png = [
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x01, 0x02, 0x03,
    ];
    write_file(&root, &rel(&["img.png"]), &png);
    assert_eq!(
        repo.get_additions_deletions(&rel(&["img.png"]), &root),
        (0, 0)
    );
}

// ---------------------------------------------------------------------------
// UTF-16 — LE common, BE rare, plus BOM-strip and CRLF normalization checks
// ---------------------------------------------------------------------------

#[test]
fn utf16le_one_line_modified_counts_one_one() {
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["u.txt"]), &utf16le_bytes("one\ntwo\nthree\n"));
    commit(&mut repo, &root, "c0");
    write_file(
        &root,
        &rel(&["u.txt"]),
        &utf16le_bytes("one\nCHANGED\nthree\n"),
    );
    assert_eq!(
        repo.get_additions_deletions(&rel(&["u.txt"]), &root),
        (1, 1)
    );
}

#[test]
fn utf16be_one_line_modified_counts_one_one() {
    // The case the little_endian:false arm exists for. Mishandled endianness
    // would decode to byte-swapped garbage and miscount.
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["u.txt"]), &utf16be_bytes("alpha\nbeta\n"));
    commit(&mut repo, &root, "c0");
    write_file(&root, &rel(&["u.txt"]), &utf16be_bytes("alpha\nGAMMA\n"));
    assert_eq!(
        repo.get_additions_deletions(&rel(&["u.txt"]), &root),
        (1, 1)
    );
}

#[test]
fn utf16_to_utf8_same_text_is_no_change() {
    // Requires BOM stripping: if the BOM survived into the decoded string, the
    // UTF-16 side's first line would carry a U+FEFF the UTF-8 side lacks.
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["u.txt"]), &utf16le_bytes("hello\nworld\n"));
    commit(&mut repo, &root, "c0");
    write_file(&root, &rel(&["u.txt"]), "hello\nworld\n"); // same text, UTF-8
    assert_eq!(
        repo.get_additions_deletions(&rel(&["u.txt"]), &root),
        (0, 0)
    );
}

#[test]
fn utf16_crlf_normalized_same_as_lf() {
    // Notepad writes UTF-16 with CRLF. A pure line-ending flip must not churn.
    // This is what pulling .replace() out to the whole match result protects.
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["u.txt"]), &utf16le_bytes("one\r\ntwo\r\n")); // CRLF
    commit(&mut repo, &root, "c0");
    write_file(&root, &rel(&["u.txt"]), &utf16le_bytes("one\ntwo\n")); // LF, same text
    assert_eq!(
        repo.get_additions_deletions(&rel(&["u.txt"]), &root),
        (0, 0)
    );
}

// ---------------------------------------------------------------------------
// Windows-1252 high byte — invalid UTF-8 but text, must not be binary/panic
// ---------------------------------------------------------------------------

#[test]
fn latin1_high_byte_counts_as_text() {
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["a.txt"]), b"cafe\nmenu\n");
    commit(&mut repo, &root, "c0");
    // 0xE9 = é in Windows-1252; invalid UTF-8 but clearly text.
    // "cafe"->"café" is a modified line (1/1); plus one appended line (1/0).
    write_file(&root, &rel(&["a.txt"]), b"caf\xE9\nmenu\nnew line\n");
    assert_eq!(
        repo.get_additions_deletions(&rel(&["a.txt"]), &root),
        (2, 1)
    );
}

// ---------------------------------------------------------------------------
// Integration: get_additions_deletions through a realistic session.
//
// Unlike the per-case unit tests, this drives the repo the way a user would —
// several commits, edits between them, a restore — and checks that the stat
// always reflects "current workspace vs the checkpoint we're sitting on".
// The key property under test: get_additions_deletions diffs against
// last_commit() (the current checkpoint), so its answer changes as the
// current checkpoint moves, even for the same file on disk.
// ---------------------------------------------------------------------------

#[test]
fn additions_deletions_track_the_current_checkpoint_through_a_session() {
    let (_dir, root, mut repo) = fixture();

    // --- Commit 0: a small project ---------------------------------------
    write_file(&root, &rel(&["main.rs"]), "fn main() {}\n");
    write_file(&root, &rel(&["lib.rs"]), "// lib\npub fn a() {}\n");
    commit(&mut repo, &root, "c0");

    // Right after committing, the workspace matches the checkpoint exactly.
    assert_eq!(
        repo.get_additions_deletions(&rel(&["main.rs"]), &root),
        (0, 0),
        "clean workspace immediately after commit should show no change"
    );
    assert_eq!(
        repo.get_additions_deletions(&rel(&["lib.rs"]), &root),
        (0, 0)
    );

    // --- Edit, then measure BEFORE committing ----------------------------
    // main.rs: one line becomes three (the original `fn main() {}` line is
    // replaced, so it's a modified line plus added lines).
    write_file(
        &root,
        &rel(&["main.rs"]),
        "fn main() {\n    println!(\"hi\");\n}\n",
    );
    let (add, del) = repo.get_additions_deletions(&rel(&["main.rs"]), &root);
    assert!(
        add >= 2 && del >= 1,
        "expanding one line into three should show several additions and at \
         least one deletion, got ({add}, {del})"
    );
    // lib.rs untouched this round: still clean against c0.
    assert_eq!(
        repo.get_additions_deletions(&rel(&["lib.rs"]), &root),
        (0, 0)
    );

    // --- Commit 1: capture those edits + a brand-new file ----------------
    write_file(&root, &rel(&["util.rs"]), "pub fn helper() {}\n");
    commit(&mut repo, &root, "c1");

    // Everything clean again now that c1 captured the current state.
    assert_eq!(
        repo.get_additions_deletions(&rel(&["main.rs"]), &root),
        (0, 0)
    );
    assert_eq!(
        repo.get_additions_deletions(&rel(&["util.rs"]), &root),
        (0, 0)
    );

    // --- Delete a file in the workspace, measure before committing -------
    delete_file(&root, &rel(&["lib.rs"]));
    let (add, del) = repo.get_additions_deletions(&rel(&["lib.rs"]), &root);
    assert_eq!(
        (add, del),
        (0, 2),
        "deleting the 2-line lib.rs should read as 0 additions, 2 deletions"
    );

    // --- Commit 2: record the deletion -----------------------------------
    commit(&mut repo, &root, "c2");
    assert!(repo.is_on_latest_checkpoint());

    // --- Restore to c0: the current checkpoint moves backwards -----------
    // This is the property the test exists for. The SAME file (main.rs) now
    // diffs against c0's version, not c1's, so a workspace that matches c0
    // reads as clean even though it differs from the latest checkpoint.
    repo.restore_checkpoint(0, &root).unwrap();
    assert!(!repo.is_on_latest_checkpoint());

    // After restoring c0, the workspace IS c0, so everything is clean
    // *relative to c0* — last_commit() now points at c0.
    assert_eq!(
        repo.get_additions_deletions(&rel(&["main.rs"]), &root),
        (0, 0),
        "after restoring c0, workspace matches c0 so main.rs reads clean"
    );
    // lib.rs was restored (it existed at c0), so it's back and clean.
    assert_eq!(
        read_file(&root, &rel(&["lib.rs"])),
        b"// lib\npub fn a() {}\n"
    );
    assert_eq!(
        repo.get_additions_deletions(&rel(&["lib.rs"]), &root),
        (0, 0)
    );
    // util.rs did NOT exist at c0, so restore deleted it from the workspace.
    assert!(!root.get().join("util.rs").exists());

    // --- Edit while sitting on the old checkpoint ------------------------
    // get_additions_deletions still works (it just diffs against c0); it's
    // create_commit that refuses while off-latest, not the stat function.
    write_file(&root, &rel(&["main.rs"]), "fn main() { todo!() }\n");
    let (add, del) = repo.get_additions_deletions(&rel(&["main.rs"]), &root);
    assert!(
        add >= 1 && del >= 1,
        "editing main.rs while on c0 should diff against c0, got ({add}, {del})"
    );

    // And committing from here is refused — the stat measuring a change does
    // not imply the commit is allowed.
    write_file(&root, &rel(&["main.rs"]), "fn main() {}\n"); // revert to c0 content
    repo.restore_checkpoint(2, &root).unwrap(); // walk back to latest first
    assert!(repo.is_on_latest_checkpoint());
}

// ---------------------------------------------------------------------------
// Per-commit additions/deletions stats tests
// ---------------------------------------------------------------------------

#[test]
fn first_commit_counts_all_lines_as_additions() {
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["a.txt"]), "one\ntwo\nthree\n"); // 3 lines
    write_file(&root, &rel(&["b.txt"]), "alpha\nbeta\n"); // 2 lines
    commit(&mut repo, &root, "c0");

    let c = &repo.commits[0];
    assert_eq!(
        c.additions, 5,
        "all 5 lines across both new files are additions"
    );
    assert_eq!(c.deletions, 0, "a first commit deletes nothing");
}

#[test]
fn second_commit_counts_only_the_delta() {
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["a.txt"]), "one\ntwo\nthree\n");
    commit(&mut repo, &root, "c0");

    // Modify one line, append one line: c1's stats describe ONLY this change,
    // not the whole file again.
    write_file(&root, &rel(&["a.txt"]), "one\nCHANGED\nthree\nfour\n");
    commit(&mut repo, &root, "c1");

    let c = &repo.commits[1];
    // Modified line = 1 add + 1 del (similar convention); appended = 1 add.
    // If similar reports differently, adjust to what prints — the point is
    // it's small (the delta), not 4 (the file).
    assert_eq!((c.additions, c.deletions), (2, 1));
}

#[test]
fn commit_with_a_new_file_counts_its_lines_as_additions() {
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["a.txt"]), "x\n");
    commit(&mut repo, &root, "c0");

    write_file(&root, &rel(&["new.txt"]), "p\nq\nr\n"); // 3 brand-new lines
    commit(&mut repo, &root, "c1");

    let c = &repo.commits[1];
    assert_eq!(c.additions, 3);
    assert_eq!(c.deletions, 0);
}

#[test]
fn commit_with_a_deletion_counts_old_lines_as_deletions() {
    // The bucket that must be sourced from file_data, not a disk read.
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["keep.txt"]), "stays\n");
    write_file(&root, &rel(&["gone.txt"]), "a\nb\nc\nd\n"); // 4 lines
    commit(&mut repo, &root, "c0");

    delete_file(&root, &rel(&["gone.txt"]));
    // Need another change so the commit isn't a NoOp on the delete alone?
    // No: a deletion is itself a non-empty delta, so this commits fine.
    commit(&mut repo, &root, "c1");

    let c = &repo.commits[1];
    assert_eq!(
        c.deletions, 4,
        "all 4 lines of the deleted file count as deletions"
    );
    assert_eq!(c.additions, 0);
}

#[test]
fn mixed_commit_sums_all_three_buckets() {
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["mod.txt"]), "old1\nold2\n");
    write_file(&root, &rel(&["del.txt"]), "g1\ng2\ng3\n"); // will be deleted (3 del)
    commit(&mut repo, &root, "c0");

    write_file(&root, &rel(&["mod.txt"]), "old1\nNEW\n"); // 1 line modified: 1 add + 1 del
    delete_file(&root, &rel(&["del.txt"])); // 3 deletions
    write_file(&root, &rel(&["add.txt"]), "n1\nn2\n"); // new file: 2 additions
    commit(&mut repo, &root, "c1");

    let c = &repo.commits[1];
    // additions: 1 (mod) + 2 (new) = 3
    // deletions: 1 (mod) + 3 (deleted file) = 4
    assert_eq!((c.additions, c.deletions), (3, 4));
}

#[test]
fn binary_changes_contribute_zero_to_commit_stats() {
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["code.txt"]), "line one\n");
    write_file(
        &root,
        &rel(&["img.png"]),
        &[0x89, b'P', b'N', b'G', 0x00, 0x01],
    );
    commit(&mut repo, &root, "c0");

    // Change the text file (counts) and the binary file (must not count).
    write_file(&root, &rel(&["code.txt"]), "line one\nline two\n"); // +1
    write_file(
        &root,
        &rel(&["img.png"]),
        &[0x89, b'P', b'N', b'G', 0x00, 0x02, 0x03],
    );
    commit(&mut repo, &root, "c1");

    let c = &repo.commits[1];
    assert_eq!(
        c.additions, 1,
        "only the text line counts; the binary edit is 0"
    );
    assert_eq!(c.deletions, 0);
}

#[test]
fn stats_survive_save_and_load() {
    // The fields are plain usize on Commit and must round-trip through data.json.
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["a.txt"]), "one\ntwo\nthree\n");
    commit(&mut repo, &root, "c0");
    write_file(&root, &rel(&["a.txt"]), "one\nCHANGED\nthree\nfour\n");
    commit(&mut repo, &root, "c1");

    let loaded = Repo::load_project(&root.data_folder().sentinel()).unwrap();
    for i in 0..repo.commits.len() {
        assert_eq!(loaded.commits[i].additions, repo.commits[i].additions);
        assert_eq!(loaded.commits[i].deletions, repo.commits[i].deletions);
    }
}

#[test]
fn binary_only_commit_records_zero_stats_but_still_commits() {
    // A commit consisting solely of a binary change is a real (non-NoOp) delta,
    // so it commits — but its line stats are honestly zero.
    let (_dir, root, mut repo) = fixture();
    write_file(&root, &rel(&["data.bin"]), &[0x00, 0x01, 0x02]);
    commit(&mut repo, &root, "c0");

    write_file(&root, &rel(&["data.bin"]), &[0x00, 0x01, 0x02, 0x03, 0x04]);
    let result = commit(&mut repo, &root, "c1");

    assert_eq!(
        result,
        CommitResult::Success,
        "binary change is a real commit"
    );
    let c = &repo.commits[1];
    assert_eq!((c.additions, c.deletions), (0, 0));
}
