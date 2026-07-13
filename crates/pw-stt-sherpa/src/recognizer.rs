//! `ReazonSpeech` (zipformer transducer) recognizer adapter.

use std::path::{Path, PathBuf};

use pw_application::speech::{PortError, SpeechRecognizer};
use sherpa_onnx::{
    OfflineModelConfig, OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig,
};

use crate::SherpaError;

const SAMPLE_RATE: i32 = 16_000;

fn preferred_model_path(dir: &Path, component: &str) -> PathBuf {
    let int8 = dir.join(format!("{component}-epoch-99-avg-1.int8.onnx"));
    if int8.is_file() {
        int8
    } else {
        dir.join(format!("{component}-epoch-99-avg-1.onnx"))
    }
}

/// File layout of the sherpa-onnx `ReazonSpeech` zipformer package.
#[derive(Debug, Clone)]
pub struct RecognizerModelPaths {
    pub encoder: PathBuf,
    pub decoder: PathBuf,
    pub joiner: PathBuf,
    pub tokens: PathBuf,
}

impl RecognizerModelPaths {
    /// Standard layout inside the extracted
    /// `sherpa-onnx-zipformer-ja-reazonspeech-2024-08-01` directory.
    #[must_use]
    pub fn in_directory(dir: &Path) -> Self {
        Self {
            encoder: preferred_model_path(dir, "encoder"),
            decoder: preferred_model_path(dir, "decoder"),
            joiner: preferred_model_path(dir, "joiner"),
            tokens: dir.join("tokens.txt"),
        }
    }
}

pub struct ReazonSpeechRecognizer {
    inner: OfflineRecognizer,
}

impl ReazonSpeechRecognizer {
    /// Loads the transducer model set.
    ///
    /// # Errors
    ///
    /// Returns [`SherpaError`] when any model file is missing or the
    /// recognizer cannot be constructed.
    pub fn new(paths: &RecognizerModelPaths) -> Result<Self, SherpaError> {
        for path in [&paths.encoder, &paths.decoder, &paths.joiner, &paths.tokens] {
            if !path.is_file() {
                return Err(SherpaError::ModelMissing(path.clone()));
            }
        }
        let config = OfflineRecognizerConfig {
            model_config: OfflineModelConfig {
                transducer: OfflineTransducerModelConfig {
                    encoder: Some(paths.encoder.to_string_lossy().into_owned()),
                    decoder: Some(paths.decoder.to_string_lossy().into_owned()),
                    joiner: Some(paths.joiner.to_string_lossy().into_owned()),
                },
                tokens: Some(paths.tokens.to_string_lossy().into_owned()),
                num_threads: 2,
                provider: Some("cpu".to_owned()),
                ..OfflineModelConfig::default()
            },
            decoding_method: Some("greedy_search".to_owned()),
            ..OfflineRecognizerConfig::default()
        };
        let inner =
            OfflineRecognizer::create(&config).ok_or(SherpaError::Init("offline recognizer"))?;
        Ok(Self { inner })
    }
}

impl SpeechRecognizer for ReazonSpeechRecognizer {
    fn transcribe(&mut self, samples: &[f32]) -> Result<String, PortError> {
        let stream = self.inner.create_stream();
        stream.accept_waveform(SAMPLE_RATE, samples);
        self.inner.decode(&stream);
        let text = stream
            .get_result()
            .map(|result| result.text)
            .unwrap_or_default();
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use pw_application::speech::SpeechRecognizer;

    use super::{ReazonSpeechRecognizer, RecognizerModelPaths};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock is before Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "pw-stt-sherpa-recognizer-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove test directory");
        }
    }

    #[test]
    fn model_paths_prefer_int8_files_when_present() {
        let dir = TestDirectory::new();
        for component in ["encoder", "decoder", "joiner"] {
            fs::write(
                dir.0.join(format!("{component}-epoch-99-avg-1.int8.onnx")),
                [],
            )
            .expect("create int8 model file");
        }

        let paths = RecognizerModelPaths::in_directory(&dir.0);

        assert_eq!(
            paths.encoder,
            dir.0.join("encoder-epoch-99-avg-1.int8.onnx")
        );
        assert_eq!(
            paths.decoder,
            dir.0.join("decoder-epoch-99-avg-1.int8.onnx")
        );
        assert_eq!(paths.joiner, dir.0.join("joiner-epoch-99-avg-1.int8.onnx"));
    }

    #[test]
    fn model_paths_fall_back_to_fp32_files_when_int8_is_missing() {
        let dir = TestDirectory::new();

        let paths = RecognizerModelPaths::in_directory(&dir.0);

        assert_eq!(paths.encoder, dir.0.join("encoder-epoch-99-avg-1.onnx"));
        assert_eq!(paths.decoder, dir.0.join("decoder-epoch-99-avg-1.onnx"));
        assert_eq!(paths.joiner, dir.0.join("joiner-epoch-99-avg-1.onnx"));
    }

    /// Integration test against the real model. Run manually:
    /// `PW_STT_MODEL_DIR=... cargo test -p pw-stt-sherpa -- --ignored`
    #[test]
    #[ignore = "requires downloaded ReazonSpeech model"]
    fn transcribes_the_bundled_japanese_sample() {
        let dir = std::env::var("PW_STT_MODEL_DIR").expect("set PW_STT_MODEL_DIR");
        let paths = RecognizerModelPaths::in_directory(Path::new(&dir));
        let mut recognizer = ReazonSpeechRecognizer::new(&paths).unwrap();

        let wave =
            sherpa_onnx::Wave::read(&Path::new(&dir).join("test_wavs/3.wav").to_string_lossy())
                .expect("read bundled test wav");
        assert_eq!(wave.sample_rate(), 16_000);
        let text = recognizer.transcribe(wave.samples()).unwrap();
        // transcript.txt: ヤンバルクイナとの出会いは１８歳の時だった。
        assert!(
            text.contains("ヤンバルクイナ"),
            "unexpected transcription: {text}"
        );
    }
}
