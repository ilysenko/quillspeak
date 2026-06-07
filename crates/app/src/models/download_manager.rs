use std::collections::HashMap;

use crate::command::{DownloadId, ModelDownloadOutcome};
use crate::models::downloader::DownloadHandle;
use crate::models::view_model::ModelStatus;

#[derive(Debug, Default)]
pub struct ModelDownloadManager {
    next_download_id: DownloadId,
    active_downloads: HashMap<String, ActiveDownload>,
    statuses: HashMap<String, ModelStatus>,
}

#[derive(Debug)]
struct ActiveDownload {
    download_id: DownloadId,
    handle: Option<DownloadHandle>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishEffect {
    Completed,
    Canceled,
    Failed(String),
    Stale,
}

impl ModelDownloadManager {
    pub fn statuses(&self) -> &HashMap<String, ModelStatus> {
        &self.statuses
    }

    pub fn begin(&mut self, model_id: &str) -> Option<DownloadId> {
        if self.active_downloads.contains_key(model_id) {
            return None;
        }

        let download_id = self.next_download_id;
        self.next_download_id = self.next_download_id.saturating_add(1);
        self.active_downloads.insert(
            model_id.to_string(),
            ActiveDownload {
                download_id,
                handle: None,
            },
        );
        self.statuses.insert(
            model_id.to_string(),
            ModelStatus::Downloading {
                downloaded: 0,
                total: None,
            },
        );
        Some(download_id)
    }

    pub fn attach_handle(
        &mut self,
        model_id: &str,
        download_id: DownloadId,
        handle: DownloadHandle,
    ) {
        let Some(active) = self.active_downloads.get_mut(model_id) else {
            return;
        };
        if active.download_id == download_id {
            active.handle = Some(handle);
        }
    }

    pub fn cancel(&mut self, model_id: &str) -> Option<DownloadHandle> {
        let active = self.active_downloads.get(model_id)?;
        let handle = active.handle.clone();
        let (downloaded, total) = self
            .statuses
            .get(model_id)
            .and_then(ModelStatus::download_progress)
            .unwrap_or((0, None));
        self.statuses.insert(
            model_id.to_string(),
            ModelStatus::Canceling { downloaded, total },
        );
        handle
    }

    pub fn progress(
        &mut self,
        download_id: DownloadId,
        model_id: &str,
        downloaded: u64,
        total: Option<u64>,
    ) -> bool {
        if !self.is_current(download_id, model_id) || self.is_canceling(model_id) {
            return false;
        }

        self.statuses.insert(
            model_id.to_string(),
            ModelStatus::Downloading { downloaded, total },
        );
        true
    }

    pub fn verifying(
        &mut self,
        download_id: DownloadId,
        model_id: &str,
        downloaded: u64,
        total: Option<u64>,
    ) -> bool {
        if !self.is_current(download_id, model_id) || self.is_canceling(model_id) {
            return false;
        }

        self.statuses.insert(
            model_id.to_string(),
            ModelStatus::Verifying { downloaded, total },
        );
        true
    }

    pub fn finish(
        &mut self,
        download_id: DownloadId,
        model_id: &str,
        outcome: ModelDownloadOutcome,
    ) -> FinishEffect {
        if !self.is_current(download_id, model_id) {
            return FinishEffect::Stale;
        }

        self.active_downloads.remove(model_id);
        match outcome {
            ModelDownloadOutcome::Completed => {
                self.statuses.remove(model_id);
                FinishEffect::Completed
            }
            ModelDownloadOutcome::Canceled => {
                self.statuses.remove(model_id);
                FinishEffect::Canceled
            }
            ModelDownloadOutcome::Failed(error) => {
                self.statuses
                    .insert(model_id.to_string(), ModelStatus::Error(error.clone()));
                FinishEffect::Failed(error)
            }
        }
    }

    pub fn is_active(&self, model_id: &str) -> bool {
        self.active_downloads.contains_key(model_id)
    }

    pub fn set_error(&mut self, model_id: &str, error: String) {
        self.statuses
            .insert(model_id.to_string(), ModelStatus::Error(error));
    }

    fn is_current(&self, download_id: DownloadId, model_id: &str) -> bool {
        self.active_downloads
            .get(model_id)
            .is_some_and(|active| active.download_id == download_id)
    }

    fn is_canceling(&self, model_id: &str) -> bool {
        matches!(
            self.statuses.get(model_id),
            Some(ModelStatus::Canceling { .. })
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL_ID: &str = "tiny";

    #[test]
    fn begin_creates_active_download() {
        let mut manager = ModelDownloadManager::default();
        let download_id = manager.begin(MODEL_ID);

        assert_eq!(download_id, Some(0));
        assert!(manager.is_active(MODEL_ID));
        assert!(matches!(
            manager.statuses().get(MODEL_ID),
            Some(ModelStatus::Downloading {
                downloaded: 0,
                total: None,
            })
        ));
    }

    #[test]
    fn duplicate_begin_is_ignored() {
        let mut manager = ModelDownloadManager::default();

        assert_eq!(manager.begin(MODEL_ID), Some(0));
        assert_eq!(manager.begin(MODEL_ID), None);
    }

    #[test]
    fn stale_progress_is_ignored() {
        let mut manager = ModelDownloadManager::default();
        let download_id = manager.begin(MODEL_ID).unwrap();

        assert!(!manager.progress(download_id + 1, MODEL_ID, 50, Some(100)));
        assert_eq!(
            manager.statuses().get(MODEL_ID),
            Some(&ModelStatus::Downloading {
                downloaded: 0,
                total: None,
            })
        );
    }

    #[test]
    fn cancel_moves_status_to_canceling() {
        let mut manager = ModelDownloadManager::default();
        let download_id = manager.begin(MODEL_ID).unwrap();
        assert!(manager.progress(download_id, MODEL_ID, 25, Some(100)));

        manager.cancel(MODEL_ID);

        assert_eq!(
            manager.statuses().get(MODEL_ID),
            Some(&ModelStatus::Canceling {
                downloaded: 25,
                total: Some(100),
            })
        );
    }

    #[test]
    fn progress_after_cancel_is_ignored() {
        let mut manager = ModelDownloadManager::default();
        let download_id = manager.begin(MODEL_ID).unwrap();
        manager.cancel(MODEL_ID);

        assert!(!manager.progress(download_id, MODEL_ID, 25, Some(100)));
        assert!(matches!(
            manager.statuses().get(MODEL_ID),
            Some(ModelStatus::Canceling { .. })
        ));
    }

    #[test]
    fn canceled_finish_returns_to_not_installed_state() {
        let mut manager = ModelDownloadManager::default();
        let download_id = manager.begin(MODEL_ID).unwrap();
        manager.cancel(MODEL_ID);

        let effect = manager.finish(download_id, MODEL_ID, ModelDownloadOutcome::Canceled);

        assert_eq!(effect, FinishEffect::Canceled);
        assert!(!manager.is_active(MODEL_ID));
        assert_eq!(manager.statuses().get(MODEL_ID), None);
    }

    #[test]
    fn failed_finish_records_error() {
        let mut manager = ModelDownloadManager::default();
        let download_id = manager.begin(MODEL_ID).unwrap();

        let effect = manager.finish(
            download_id,
            MODEL_ID,
            ModelDownloadOutcome::Failed("network failed".to_string()),
        );

        assert_eq!(effect, FinishEffect::Failed("network failed".to_string()));
        assert_eq!(
            manager.statuses().get(MODEL_ID),
            Some(&ModelStatus::Error("network failed".to_string()))
        );
    }

    #[test]
    fn completed_finish_clears_transient_status() {
        let mut manager = ModelDownloadManager::default();
        let download_id = manager.begin(MODEL_ID).unwrap();

        let effect = manager.finish(download_id, MODEL_ID, ModelDownloadOutcome::Completed);

        assert_eq!(effect, FinishEffect::Completed);
        assert!(!manager.is_active(MODEL_ID));
        assert_eq!(manager.statuses().get(MODEL_ID), None);
    }
}
