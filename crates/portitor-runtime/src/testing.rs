// SPDX-FileCopyrightText: Portitor contributors
//
// SPDX-License-Identifier: Apache-2.0

//! Helpers shared by the tests of this crate and of the binaries
//! (feature `test-support`): settings from a list instead of the
//! environment, and a temporary directory.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::system::new_uuid;

/// A lookup of settings: a setting's name gives its value, or `None`
/// when it is not set.
pub type Lookup = Box<dyn Fn(&str) -> Option<OsString>>;

/// A lookup of settings in `pairs` (name, value), in the shape the
/// `from_vars` functions take instead of `std::env::var_os`. A name not
/// in `pairs` is not set; a name given twice has its last value, so a
/// test can change one setting of a list by pushing it again. The lookup
/// keeps its own copy of `pairs`.
#[must_use]
pub fn vars(pairs: &[(&str, &str)]) -> Lookup {
    let mut owned: Vec<(String, OsString)> = Vec::new();
    for (name, value) in pairs {
        owned.push(((*name).to_owned(), OsString::from(value)));
    }
    Box::new(move |name| {
        // from the end: the last value of a name wins
        for (known, value) in owned.iter().rev() {
            if known == name {
                return Some(value.clone());
            }
        }
        None
    })
}

/// A new, empty directory under the system's temporary directory,
/// removed with its content when this value is dropped.
#[derive(Debug)]
pub struct TempDir(PathBuf);

impl TempDir {
    /// A new directory whose name starts with `portitor-<prefix>-`.
    ///
    /// # Panics
    /// When the directory cannot be made.
    #[must_use]
    pub fn new(prefix: &str) -> Self {
        let id = new_uuid();
        let name = format!("portitor-{prefix}-{}", id.trim_start_matches("urn:uuid:"));
        let dir = std::env::temp_dir().join(name);
        if let Err(error) = std::fs::create_dir_all(&dir) {
            panic!("cannot make the test directory {}: {error}", dir.display());
        }
        Self(dir)
    }

    /// The directory.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Writes `text` to the file `name` in the directory, and returns its
    /// path.
    ///
    /// # Panics
    /// When the file cannot be written.
    #[must_use]
    pub fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        if let Err(error) = std::fs::write(&path, text) {
            panic!("cannot write the test file {}: {error}", path.display());
        }
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // a directory left behind is harmless, so a failure is ignored
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vars_finds_only_the_names_given() {
        let read = vars(&[("A", "0"), ("B", ""), ("A", "1")]);
        assert_eq!(read("A"), Some(OsString::from("1")));
        assert_eq!(read("B"), Some(OsString::new()));
        assert_eq!(read("C"), None);
    }

    #[test]
    fn a_temp_dir_is_removed_when_dropped() {
        let dir = TempDir::new("testing");
        let file = dir.write("a.txt", "a");
        assert!(file.is_file());
        let path = dir.path().to_owned();
        drop(dir);
        assert!(!path.exists());
    }
}
