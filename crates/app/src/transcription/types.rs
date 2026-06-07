use std::path::PathBuf;

use shared::{AudioInputRef, ComputeBackend, OutputAction};

use crate::audio::CapturedAudio;

#[derive(Debug, Clone)]
pub struct TranscriptionPlan {
    pub shortcut_id: String,
    pub shortcut_name: String,
    pub model_id: String,
    pub model_path: PathBuf,
    pub language: String,
    pub compute_backend: ComputeBackend,
    pub output: OutputAction,
    pub input: AudioInputRef,
}

impl TranscriptionPlan {
    pub fn into_request(self, audio: CapturedAudio) -> TranscriptionRequest {
        TranscriptionRequest {
            shortcut_id: self.shortcut_id,
            shortcut_name: self.shortcut_name,
            model_id: self.model_id,
            model_path: self.model_path,
            language: self.language,
            compute_backend: self.compute_backend,
            output: self.output,
            audio,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TranscriptionRequest {
    pub shortcut_id: String,
    pub shortcut_name: String,
    pub model_id: String,
    pub model_path: PathBuf,
    pub language: String,
    pub compute_backend: ComputeBackend,
    pub output: OutputAction,
    pub audio: CapturedAudio,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptionSegment {
    pub start_centiseconds: i64,
    pub end_centiseconds: i64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptionDebugInfo {
    pub shortcut_name: String,
    pub model_id: String,
    pub model_path: PathBuf,
    pub language: String,
    pub compute_backend: ComputeBackend,
    pub output_label: String,
    pub input_label: String,
    pub capture_duration_ms: u128,
    pub source_sample_rate: u32,
    pub source_channels: u16,
    pub source_frames: usize,
    pub dropped_samples: u64,
    pub missed_audio_chunks: u64,
    pub audio_rms: f32,
    pub audio_peak: f32,
    pub whisper_sample_rate: u32,
    pub whisper_samples: usize,
    pub inference_duration_ms: u128,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptionResult {
    pub text: String,
    pub segments: Vec<TranscriptionSegment>,
    pub output: OutputAction,
    pub debug: TranscriptionDebugInfo,
}
