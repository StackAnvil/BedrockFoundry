//! Shared checks for the Bedrock Foundry fuzz targets.

use std::{
    fs,
    path::{Component, Path, PathBuf},
};

/// Accepted ZIP members must not exceed the production expansion limit.
pub const MAX_EXPANDED: u64 = 512 * 1024 * 1024;

/// Asserts that an accepted member name can only name a path below its destination.
pub fn assert_relative(name: &str) {
    assert!(
        !name.is_empty()
            && Path::new(name)
                .components()
                .all(|c| matches!(c, Component::Normal(_))),
        "accepted member {name:?} can name a path outside its destination"
    );
}

/// A temporary directory tree with the extraction destination nested two levels
/// deep, so a member that climbs out of the destination lands inside the sandbox.
pub struct Sandbox {
    dir: tempfile::TempDir,
    pub dest: PathBuf,
}

impl Sandbox {
    #[must_use]
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("create fuzz sandbox");
        let parent = dir.path().join("outer").join("inner");
        fs::create_dir_all(&parent).expect("create fuzz sandbox parents");
        let dest = parent.join("dest");
        Self { dir, dest }
    }

    /// Asserts that every entry in the sandbox is inside one of the `allowed`
    /// directories or one of their ancestors, and that extraction created no
    /// symbolic links.
    pub fn assert_contained(&self, allowed: &[PathBuf]) {
        let mut pending = vec![self.dir.path().to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in fs::read_dir(&directory).expect("read fuzz sandbox") {
                let path = entry.expect("read fuzz sandbox entry").path();
                let kind = fs::symlink_metadata(&path)
                    .expect("inspect fuzz sandbox entry")
                    .file_type();
                assert!(
                    !kind.is_symlink(),
                    "extraction created a symlink at {path:?}"
                );
                assert!(
                    allowed
                        .iter()
                        .any(|dir| path.starts_with(dir) || dir.starts_with(&path)),
                    "extraction wrote {path:?} outside {allowed:?}"
                );
                if kind.is_dir() {
                    pending.push(path);
                }
            }
        }
    }
}

impl Default for Sandbox {
    fn default() -> Self {
        Self::new()
    }
}
