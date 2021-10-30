use serde::{Deserialize, Serialize};
use vid_dup_finder_lib::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CachedVideoData {
    pub hash: VideoHash,
    pub stats: VideoStats,
}

//loss of space is acceptable on the assmption that most of the time we try and
//load a video, the load will probably succeed.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Serialize, Deserialize)]

pub struct CacheEntry(pub Result<CachedVideoData, HashCreationErrorKind>);

impl From<Result<(VideoHash, VideoStats), HashCreationErrorKind>> for CacheEntry {
    fn from(x: Result<(VideoHash, VideoStats), HashCreationErrorKind>) -> Self {
        match x {
            Ok((hash, stats)) => CacheEntry(Ok(CachedVideoData { hash, stats })),
            Err(e) => CacheEntry(Err(e)),
        }
    }
}
