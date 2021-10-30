use std::{
    collections::{hash_map::RandomState, HashSet},
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    result::Result,
};

use itertools::Itertools;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use walkdir::WalkDir;

/// Errors encountered during the file enumeration process.
#[derive(Error, Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum FileProjectionError {
    /// A src_path or excl_path could not be read from the filesystem.
    #[error("Path not found: {0}")]
    PathNotFound(PathBuf),

    #[error("Excl path not found: {0}")]
    ExclPathNotFound(PathBuf),

    /// Any Io error that occurred during file enumeration.
    #[error("File enumeration failed")]
    Enumeration(String),

    /// a src_path is excluded by an excl_path.
    #[error("A start path is excluded by an excl path")]
    SrcPathExcluded { src_path: PathBuf, excl_path: PathBuf },
}

impl From<walkdir::Error> for FileProjectionError {
    fn from(e: walkdir::Error) -> Self {
        Self::Enumeration(format!("{}", e))
    }
}

impl From<&walkdir::Error> for FileProjectionError {
    fn from(e: &walkdir::Error) -> Self {
        Self::Enumeration(format!("{}", e))
    }
}

#[derive(Debug, Copy, Clone, Hash, Eq, PartialEq, Ord, PartialOrd)]
enum FileProjectionState {
    Unprojected,
    ProjectedUsingFs,
    ProjectedUsingList,
}

use FileProjectionState::*;

/// A utility struct for holding a set of paths, and all children from those paths.
/// Contains an associated set of "exclude" paths whose children should not be returned.
#[derive(Debug, Clone)]
pub struct FileProjection {
    src_paths: Vec<PathBuf>,
    excl_paths: Vec<PathBuf>,
    projected_files: HashSet<PathBuf>,
    state: FileProjectionState,
    excl_exts: Vec<OsString>,
}

impl FileProjection {
    /// Create a new FileProjection with the given src_paths, excl_paths and ignore-extensions
    /// Child files can either be got by projecting the src_paths, either from
    /// the filesystem (project_using_fs), or from some list (project_using_list).
    /// Once projection has occurred the projected files will be cached by this struct
    /// (This feature is mostly to avoid having to visit the filesystem more than once
    /// when performing large projections)
    ///
    /// Projected files can be retrieved by calling [projected_files][Self::projected_files]
    pub fn new(
        src_paths: impl IntoIterator<Item = impl AsRef<Path>>,
        excl_paths: impl IntoIterator<Item = impl AsRef<Path>>,
        excl_exts: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> Result<Self, FileProjectionError> {
        let src_paths = src_paths
            .into_iter()
            .map(|p| p.as_ref().to_path_buf())
            .collect::<Vec<_>>();

        let excl_paths = excl_paths
            .into_iter()
            .map(|p| p.as_ref().to_path_buf())
            .collect::<Vec<_>>();

        //check that the same path does not appear in srcs and excls
        excl_paths
            .iter()
            .find_map(|excl_path| {
                src_paths.iter().find_map(|src_path| {
                    src_path
                        .starts_with(excl_path)
                        .then(|| (src_path.to_path_buf(), excl_path.to_path_buf()))
                })
            })
            .map(|(src_path, excl_path)| {
                <Result<(), _>>::Err(FileProjectionError::SrcPathExcluded { src_path, excl_path })
            })
            .transpose()?;

        Ok(Self {
            src_paths,
            excl_paths,
            projected_files: Default::default(),
            state: Unprojected,
            excl_exts: excl_exts.into_iter().map(|x| x.as_ref().to_os_string()).collect(),
        })
    }

    /// Returns true if the given path is a child of any src_path,
    /// and is not a child of any excl_path.
    pub fn contains(&self, src_path: impl AsRef<Path>) -> bool {
        self.raw_includes(&src_path) && !self.raw_excludes(&src_path)
    }

    fn raw_includes(&self, p: impl AsRef<Path>) -> bool {
        self.src_paths.iter().any(|src_path| p.as_ref().starts_with(src_path))
    }

    fn raw_excludes(&self, p: impl AsRef<Path>) -> bool {
        self.excl_paths
            .iter()
            .any(|excl_path| p.as_ref().starts_with(excl_path))
    }

    /// Visit the filesystem to get all child files which are a child of any of Self::src_paths,
    /// and which are not a child of Self::excl_paths.
    ///
    /// # Return values
    /// Returns Err() if any path in Self::src_paths or Self::excl_paths cannot be read from
    /// the filesystem.
    ///
    /// Otherwise returns Ok() containing a list of all other errors encountered while retrieving
    /// paths from the filesystem.
    ///
    /// # Panics
    /// This function will panic if either project_using_fs or project_using_list
    /// has already been called.
    pub fn project_using_fs(&mut self) -> Result<Vec<walkdir::Error>, FileProjectionError> {
        use FileProjectionError::*;

        match self.state {
            //if we have previously projected using a list, then forbid projection from the filesystem.
            ProjectedUsingList => {
                panic!("FileProjection::project_using_fs called, but projection has already been done using list")
            }
            // Otherwise if we have already projected, then there is nothing to do..
            ProjectedUsingFs => Ok(vec![]),

            Unprojected => {
                //we will return a fatal error if any directory/file that the user
                //has specified does not exist.
                for path in &self.src_paths {
                    if !path.exists() {
                        return Err(PathNotFound(path.to_owned()));
                    }
                }

                for path in &self.excl_paths {
                    if !path.exists() {
                        return Err(ExclPathNotFound(path.to_owned()));
                    }
                }

                let (enumerated_paths, loading_errs): (HashSet<PathBuf, RandomState>, Vec<walkdir::Error>) = self
                    .src_paths
                    .iter()
                    .flat_map(|src_path| {
                        WalkDir::new(src_path).into_iter().filter_entry(|entry| {
                            let src_path = entry.path();
                            self.contains(src_path) && !self.has_ignore_ext(src_path)
                        })
                    })
                    .filter_map(|dir_entry_res| match dir_entry_res {
                        Err(e) => Some(Err(e)),
                        Ok(dir_entry) => {
                            let src_path = dir_entry.path();
                            if src_path.is_file() {
                                Some(Ok(src_path.to_path_buf()))
                            } else {
                                None
                            }
                        }
                    })
                    .partition_result();

                self.projected_files = enumerated_paths;
                self.state = ProjectedUsingFs;

                Ok(loading_errs)
            }
        }
    }

    /// Enumerate files by filtering a list of paths.
    ///
    /// # Panics
    /// This function will panic if either project_using_fs or project_using_list
    /// has already been called.
    pub fn project_using_list(&mut self, list: impl IntoIterator<Item = impl AsRef<Path>>) {
        match self.state {
            //if we have previously projected using the fs, then forbig projection from a list.
            ProjectedUsingFs => {
                panic!("FileProjection::project_using_list called, but projection has already been done using fs")
            }
            // Otherwise if we have already projected, then return the projection.
            ProjectedUsingList => (),
            Unprojected => {
                self.projected_files = list
                    .into_iter()
                    .filter(|p| self.contains(p))
                    .map(|x| x.as_ref().to_path_buf())
                    .collect();

                self.state = ProjectedUsingList;
            }
        }
    }

    /// Obtain the set of all enumerated files. File enumeration must have already
    /// taken place.
    ///
    /// # Panics
    /// This function will panic if enumeration has not occurred.
    pub fn projected_files(&self) -> &HashSet<PathBuf> {
        match self.state {
            Unprojected => panic!("FileProjection::projected_files called without have first projected. Call project_using_fs or project_using_fs first."),
            ProjectedUsingFs |
            ProjectedUsingList => &self.projected_files,
        }
    }

    fn has_ignore_ext(&self, src_path: &Path) -> bool {
        self.excl_exts
            .iter()
            .any(|ext| src_path.extension().unwrap_or_default().eq_ignore_ascii_case(ext))
    }
}
