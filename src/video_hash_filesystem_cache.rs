use std::{
    collections::{hash_map::RandomState, HashSet},
    path::{Path, PathBuf},
};

use generic_filesystem_cache::*;
#[cfg(feature = "parallel_loading")]
use rayon::prelude::*;
use vid_dup_finder_lib::*;

use super::{cache_entry::CachedVideoData, generic_cache_if::GenericCacheIf};
use crate::*;
/// A disk-backed cache for hashes of videos on the filesystem.
/// This is a utility struct for long term storage of [VideoHashes][vid_dup_finder_lib::VideoHash].
/// The cache tracks modification times of the underlying video files, and will automatically
/// recalculate hashes based on this.
///
/// Cache entries are created and retrieved by calling [fetch_update][`VideoHashFilesystemCache::fetch_update`] with the path to a video
/// on disk. If there is no entry in the cache, or the modification time of the video is newer then
/// the cache will create a video hash for the underlying file. If the video is already cached then
/// the cache will supply its cached data
///
/// Hashes can be obtained from the cache without visiting the underlying video on the filesystem with
/// [fetch][`VideoHashFilesystemCache::fetch`].
///
/// To update all hashes within a given directory (or set of directories) use [update_using_fs][`VideoHashFilesystemCache::update_using_fs`]
///
/// # A note on interior mutability
/// All methods on this struct and its [underlying implementation][generic_filesystem_cache::ProcessingFsCache] are use
/// interior mutability allow for operations to occur in parallel.
pub struct VideoHashFilesystemCache(ProcessingFsCache<GenericCacheIf>);

impl VideoHashFilesystemCache {
    /// Load a VideoHash cache from disk the specified path. If no cache exists at cache_path
    /// then a new cache will be created.
    ///
    /// The cache will automatically save its contents to disk when cache_save_threshold write/delete
    /// operations have occurred to the cache.
    ///
    /// Note: The cache does not automatically save its contents when it goes out of scope. You must manually
    /// call [save][`VideoHashFilesystemCache::save`] after you have made the last modification to the chache contents.
    ///
    /// Returns an error if it was not possible to load the cache or create a new one.
    pub fn new(cache_save_thresold: u32, cache_path: PathBuf) -> Result<Self, VdfCacheError> {
        let interface = GenericCacheIf::new();

        let ret = ProcessingFsCache::new(cache_save_thresold, cache_path, interface)?;
        Ok(Self(ret))
    }

    /// Fetch the hash for the video file at the given source path. If the cache does not already contain a hash
    /// will not create one. This method does not read ``src_path`` on the filesystem.
    ///
    /// Returns an error if the cache has no entry for `src_path` .
    pub fn fetch(&self, src_path: impl AsRef<Path>) -> Result<VideoHash, VdfCacheError> {
        match self.fetch_entry(src_path)?.0 {
            Ok(CachedVideoData { hash, stats: _stats }) => Ok(hash),
            Err(e) => Err(VdfCacheError::from(e)),
        }
    }

    #[doc(hidden)]
    /// Utility function specifically for the example video_dup_finder GUI app. Returns additional information
    /// used to help guide manual deduplication.
    pub fn fetch_stats(&self, src_path: impl AsRef<Path>) -> Result<VideoStats, VdfCacheError> {
        match self.fetch_entry(src_path)?.0 {
            Ok(CachedVideoData { hash: _hash, stats }) => Ok(stats),
            Err(e) => Err(VdfCacheError::from(e)),
        }
    }

    /// Get the paths of all [VideoHashes][VideoHash] stored in the cache.
    pub fn all_cached_paths(&self) -> Vec<PathBuf> {
        self.0
            .keys()
            .into_iter()
            .filter(|src_path| self.fetch(src_path).is_ok())
            .collect()
    }

    /// If ``src_path`` has not been modified since it was cached, then return the cached hash.
    /// If ``src_path`` has been deleted, then remove it from the cache and return None.
    /// Otherwise create a new hash, insert it into the cache, and return it.
    ///
    /// Returns an error if it was not possible to generate a hash from `src_path`.
    pub fn fetch_update(
        &self,
        src_path: impl AsRef<Path>,
    ) -> Result<Option<Result<VideoHash, HashCreationErrorKind>>, VdfCacheError> {
        match self.0.fetch_update(&src_path.as_ref().to_path_buf()) {
            Ok(None) => Ok(None),
            Ok(Some(entry)) => match entry.0 {
                Ok(entry) => Ok(Some(Ok(entry.hash))),
                Err(hash_creation_err) => Ok(Some(Err(hash_creation_err))),
            },
            Err(cache_error) => Err(VdfCacheError::from(cache_error)),
        }
    }

    /// Save the cache to disk.
    ///
    ///Returns an error if it was not possible to write the cache to disk.
    pub fn save(&self) -> Result<(), VdfCacheError> {
        self.0.save().map_err(VdfCacheError::from)
    }

    /// For all files on the filesystem matching ``file_projection``, update the cache for all new or modified files.
    /// Also, remove items from the cache if they no longer exist in the underlying filesystem.
    ///
    /// # Return values
    /// This function will return ``Err`` if any fatal error occurs. Otherwise, it returns a group
    /// of nonfatal errors, typically a list of paths for which a [`VideoHash`] could not be generated.
    ///
    /// ## Fatal errors
    ///    * Unable to read any of the starting directories in ``file_projection``
    ///    * Any Io error when reading/writing to the cache file itself.
    ///
    /// ## Nonfatal errors
    ///    * Failure to create a hash from any individual file.
    ///    * Failure to remove an item from the cache (This is unlikely and should only occur if
    ///      calling this function more than once at the same time with overlapping paths)
    ///
    /// # Parallelism
    /// To speed up loading there is a cargo feature to allow hashes to be created from videos in parallel.
    /// Parallel loading is much faster than sequential loading but be aware that since Ffmpeg is already multithreaded
    /// this can use up a lot of CPU time.
    pub fn update_using_fs(&self, file_projection: &FileProjection) -> Result<Vec<VdfCacheError>, VdfCacheError> {
        let mut errs_ret = vec![];

        let cached_paths_in_projection = self
            .all_cached_paths()
            .into_iter()
            .filter(|src_path| file_projection.contains(src_path));

        let all_update_paths_iter = cached_paths_in_projection.chain(file_projection.projected_files().iter().cloned());

        let all_update_paths = all_update_paths_iter
            .map(PathBuf::from)
            .collect::<HashSet<_, RandomState>>();

        //Delete those items which have disappeared from the filesystem,
        // and add what's new.
        #[cfg(feature = "parallel_loading")]
        errs_ret.par_extend(
            all_update_paths
                .par_iter()
                .filter_map(|path| match self.fetch_update(&path) {
                    Ok(Some(Err(e))) => Some(VdfCacheError::from(e)),
                    Err(e) => Some(e),
                    _ => None,
                }),
        );

        #[cfg(not(feature = "parallel_loading"))]
        errs_ret.extend(
            all_update_paths
                .iter()
                .filter_map(|path| match self.fetch_update(&path) {
                    Ok(Some(Err(e))) => Some(VdfCacheError::from(e)),
                    Err(e) => Some(e),
                    _ => None,
                }),
        );
        Ok(errs_ret)
    }

    fn fetch_entry(&self, src_path: impl AsRef<Path>) -> Result<CacheEntry, VdfCacheError> {
        self.0
            .fetch(src_path.as_ref().to_path_buf())
            .map_err(VdfCacheError::from)
    }
}
