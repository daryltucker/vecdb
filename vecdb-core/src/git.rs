/*
 * PURPOSE:
 *   Helper module for interacting with Git repositories.
 *   Provides functions to detect repo status and retrieve commit SHAs.
 */

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Checks if the given path is inside a git repository.
/// Returns the root path of the repository if true.
pub fn get_git_root(path: &Path) -> Result<Option<PathBuf>> {
    let target_dir = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };

    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(target_dir)
        .output();

    match output {
        Ok(out) => {
            if out.status.success() {
                let root = String::from_utf8(out.stdout)?.trim().to_string();
                Ok(Some(PathBuf::from(root)))
            } else {
                Ok(None)
            }
        }
        Err(_) => Ok(None), // Git not installed or command failed
    }
}

/// Retrieves the current HEAD SHA for the repository containing the path.
pub fn get_head_sha(path: &Path) -> Result<Option<String>> {
    let target_dir = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };

    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(target_dir)
        .output();

    match output {
        Ok(out) => {
            if out.status.success() {
                let sha = String::from_utf8(out.stdout)?.trim().to_string();
                Ok(Some(sha))
            } else {
                Ok(None)
            }
        }
        Err(_) => Ok(None),
    }
}

/// Represents a temporary sandboxed clone of a repository
pub struct GitSandbox {
    path: PathBuf,
}

impl GitSandbox {
    /// Creates a new sandbox by cloning the repo at `repo_url` (or path) and checking out `git_ref`.
    pub fn new(repo_path: &str, git_ref: &str) -> Result<Self> {
        let temp_dir = std::env::temp_dir();
        let uuid = uuid::Uuid::new_v4();
        let sandbox_path = temp_dir.join(format!("vecdb-sandbox-{}", uuid));

        if crate::output::OUTPUT.is_interactive {
            eprintln!("Creating sandbox at {:?}", sandbox_path);
        }

        // 1. Clone
        let repo_url = sandbox_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Sandbox path contains invalid UTF-8"))?;

        let status = Command::new("git")
            .args(["clone", repo_path, repo_url])
            .status()?;

        if !status.success() {
            return Err(anyhow::anyhow!("Failed to clone repository"));
        }

        // 2. Checkout
        let status = Command::new("git")
            .args(["checkout", git_ref])
            .current_dir(&sandbox_path)
            .status()?;

        if !status.success() {
            // Cleanup if checkout fails
            let _ = std::fs::remove_dir_all(&sandbox_path);
            return Err(anyhow::anyhow!("Failed to checkout ref: {}", git_ref));
        }

        Ok(Self { path: sandbox_path })
    }

    /// Returns the path to the sandboxed repository
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for GitSandbox {
    fn drop(&mut self) {
        if crate::output::OUTPUT.is_interactive {
            eprintln!("Cleaning up sandbox at {:?}", self.path);
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
