use std::path::PathBuf;

use shared::{AudioInputRef, ComputeBackend, OutputAction};

use crate::audio::CapturedAudio;

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptionPlan {
    pub recording_id: u64,
    pub shortcut_id: String,
    pub shortcut_name: String,
    pub model_id: String,
    pub model_path: PathBuf,
    pub language: String,
    pub compute_backend: ComputeBackend,
    pub mute_output_while_recording: bool,
    pub beep_on_recording: bool,
    pub beep_volume_percent: u8,
    pub output: OutputAction,
    pub input: AudioInputRef,
}

impl TranscriptionPlan {
    pub fn into_request(self, audio: CapturedAudio) -> TranscriptionRequest {
        TranscriptionRequest {
            recording_id: self.recording_id,
            shortcut_id: self.shortcut_id,
            shortcut_name: self.shortcut_name,
            model_id: self.model_id,
            model_path: self.model_path,
            language: self.language,
            compute_backend: self.compute_backend,
            beep_on_recording: self.beep_on_recording,
            beep_volume_percent: self.beep_volume_percent,
            output: self.output,
            audio,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptionRequest {
    pub recording_id: u64,
    pub shortcut_id: String,
    pub shortcut_name: String,
    pub model_id: String,
    pub model_path: PathBuf,
    pub language: String,
    pub compute_backend: ComputeBackend,
    pub beep_on_recording: bool,
    pub beep_volume_percent: u8,
    pub output: OutputAction,
    pub audio: CapturedAudio,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptionSegment {
    pub start_centiseconds: i64,
    pub end_centiseconds: i64,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptionSkipReason {
    CaptureTooShort,
    PreparedAudioTooShort,
    NearSilent,
}

impl TranscriptionSkipReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::CaptureTooShort => {
                "captured audio is shorter than the minimum transcription duration"
            }
            Self::PreparedAudioTooShort => {
                "prepared whisper audio is shorter than the minimum transcription duration"
            }
            Self::NearSilent => "captured audio is near silent",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptionStatus {
    Completed,
    Skipped { reason: TranscriptionSkipReason },
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
    pub capture_wall_duration_ms: u128,
    pub startup_latency_ms: u128,
    pub first_callback_latency_ms: Option<u128>,
    pub audio_callback_count: u64,
    pub source_sample_rate: u32,
    pub source_channels: u16,
    pub source_frames: usize,
    pub dropped_samples: u64,
    pub missed_audio_chunks: u64,
    pub stale_callback_count: u64,
    pub stale_samples: u64,
    pub audio_rms: f32,
    pub audio_peak: f32,
    pub whisper_sample_rate: u32,
    pub whisper_samples: usize,
    pub prepared_duration_ms: u128,
    pub inference_duration_ms: u128,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptionResult {
    pub status: TranscriptionStatus,
    pub text: String,
    pub segments: Vec<TranscriptionSegment>,
    pub output: OutputAction,
    pub debug: TranscriptionDebugInfo,
}
