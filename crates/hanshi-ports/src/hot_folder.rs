//! Hot folder port — writes a PDF + sidecar settings file into the VW
//! watched input folder (SMB-mounted from the Windows box over the
//! tailnet in the real deployment; an in-process map in mocks).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use hanshi_domain::{JobId, Timestamp};

use crate::error::PortResult;

/// A file the hanshi service has staged for VW to pick up.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmittedFile {
    pub job_id: JobId,
    /// Path as VW sees it, relative to the hot folder root.
    pub relative_path: String,
    pub bytes: u64,
    pub sha256_hex: String,
    pub written_at: Timestamp,
}

#[async_trait]
pub trait HotFolderWriter: Send + Sync {
    /// Stage `bytes` as a PDF for the given job. Returns metadata about
    /// the landed file.
    async fn stage_pdf(
        &self,
        job_id: JobId,
        bytes: &[u8],
        filename_hint: Option<&str>,
    ) -> PortResult<SubmittedFile>;

    /// Is the file still sitting in the hot folder (unclaimed by VW)?
    async fn is_present(&self, submitted: &SubmittedFile) -> PortResult<bool>;

    /// Count of files currently in the hot folder (any job).
    async fn queue_depth(&self) -> PortResult<usize>;
}
