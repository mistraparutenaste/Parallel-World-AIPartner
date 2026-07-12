//! Speech capture use-case: frames in, filtered transcripts out.

mod pipeline;
mod ports;

pub use pipeline::{PipelineDiagnostics, SpeechPipeline, SpeechPipelineConfig, run_pipeline};
pub use ports::{
    FrameRead, PortError, SpeechEvents, SpeechFrameSource, SpeechRecognizer, VoiceActivityDetector,
};
