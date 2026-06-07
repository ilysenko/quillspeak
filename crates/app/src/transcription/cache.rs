use std::path::{Path, PathBuf};

use shared::ComputeBackend;

use crate::transcription::types::TranscriptionRequest;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct ModelCacheKey {
    pub(super) model_path: PathBuf,
    pub(super) compute_backend: ComputeBackend,
}

impl ModelCacheKey {
    pub(super) fn from_request(request: &TranscriptionRequest) -> Self {
        Self {
            model_path: request.model_path.clone(),
            compute_backend: request.compute_backend,
        }
    }
}

pub(super) struct CachedModel<T> {
    pub(super) key: ModelCacheKey,
    value: T,
}

pub(super) struct SingleModelCache<T> {
    entry: Option<CachedModel<T>>,
}

impl<T> SingleModelCache<T> {
    pub(super) fn get(&self, key: &ModelCacheKey) -> Option<&T> {
        self.entry
            .as_ref()
            .filter(|entry| entry.key == *key)
            .map(|entry| &entry.value)
    }

    pub(super) fn replace(&mut self, key: ModelCacheKey, value: T) -> Option<CachedModel<T>> {
        self.entry.replace(CachedModel { key, value })
    }

    pub(super) fn take_if_path(&mut self, model_path: &Path) -> Option<CachedModel<T>> {
        if self
            .entry
            .as_ref()
            .is_some_and(|entry| entry.key.model_path == model_path)
        {
            self.entry.take()
        } else {
            None
        }
    }

    pub(super) fn clear(&mut self) -> Option<CachedModel<T>> {
        self.entry.take()
    }
}

impl<T> Default for SingleModelCache<T> {
    fn default() -> Self {
        Self { entry: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_key(path: &str, compute_backend: ComputeBackend) -> ModelCacheKey {
        ModelCacheKey {
            model_path: PathBuf::from(path),
            compute_backend,
        }
    }

    #[test]
    fn single_model_cache_keeps_only_latest_context() {
        let mut cache = SingleModelCache::default();
        let first = cache_key("/tmp/first.bin", ComputeBackend::Cpu);
        let second = cache_key("/tmp/second.bin", ComputeBackend::Cpu);

        assert!(cache.replace(first.clone(), "first").is_none());
        assert_eq!(cache.get(&first), Some(&"first"));

        let evicted = cache.replace(second.clone(), "second").expect("old entry");
        assert_eq!(evicted.key, first);
        assert_eq!(cache.get(&second), Some(&"second"));
    }

    #[test]
    fn single_model_cache_takes_matching_model_path() {
        let mut cache = SingleModelCache::default();
        let key = cache_key("/tmp/model.bin", ComputeBackend::Auto);
        cache.replace(key.clone(), "context");

        assert!(cache.take_if_path(Path::new("/tmp/other.bin")).is_none());
        assert_eq!(
            cache
                .take_if_path(Path::new("/tmp/model.bin"))
                .map(|entry| entry.key),
            Some(key)
        );
        assert!(
            cache
                .get(&cache_key("/tmp/model.bin", ComputeBackend::Auto))
                .is_none()
        );
    }
}
