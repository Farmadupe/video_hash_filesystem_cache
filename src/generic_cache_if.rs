use std::path::Path;

use generic_filesystem_cache::*;
use vid_dup_finder_lib::*;

use crate::*;

pub struct GenericCacheIf {}

impl GenericCacheIf {
    pub fn new() -> Self {
        Self {}
    }
}

impl CacheInterface for GenericCacheIf {
    type T = CacheEntry;

    fn load(&self, src_path: impl AsRef<Path>) -> Self::T {
        let new_entry = VideoHash::from_path_with_stats(src_path);

        match &new_entry {
            Ok((hash, _stats)) => info!(target: "hash_creation",
                "inserting : {}",
                hash.src_path().display()
            ),
            Err(HashCreationErrorKind::DetermineVideo { src_path, error }) => warn!(target: "hash_creation",
                    "not sure if video : {}. Error: {}",
                    src_path.display(),
                    error
            ),
            Err(HashCreationErrorKind::VideoLength(src_path)) => warn!(target: "hash_creation",
                    "Too short : {}",
                    src_path.display(),

            ),
            Err(HashCreationErrorKind::VideoProcessing { src_path, error }) => warn!(target: "hash_creation",
                    "Proc err  : {} -- {}",
                    src_path.display(),
                    error
            ),
        }

        CacheEntry::from(new_entry)
    }
}
