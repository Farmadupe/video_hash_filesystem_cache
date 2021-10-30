use generic_filesystem_cache::*;
use thiserror::Error;
use vid_dup_finder_lib::*;

/// Errors occurring while inserting or removing an item from a cache.
#[derive(Error, Debug)]
pub enum VdfCacheError {
    /// An error occurred when creating a [VideoHash][vid_dup_finder_lib::VideoHash]
    #[error(transparent)]
    CreateHashError(#[from] HashCreationErrorKind),

    /// An caching error occurred.
    #[error(transparent)]
    CacheErrror(#[from] FsCacheErrorKind),
}
