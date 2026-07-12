//! Streaming resampler to the 16 kHz mono STT format.

use rubato::{FastFixedIn, PolynomialDegree, Resampler};

const CHUNK_SIZE: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum ResampleError {
    #[error("failed to construct resampler: {0}")]
    Construction(#[from] rubato::ResamplerConstructionError),
    #[error("failed to resample: {0}")]
    Process(#[from] rubato::ResampleError),
}

/// Streaming mono resampler with internal chunking. Pass-through
/// when input and output rates match.
pub struct MonoResampler {
    inner: Option<FastFixedIn<f32>>,
    pending: Vec<f32>,
}

impl MonoResampler {
    /// Creates a resampler from `input_rate` to `output_rate` Hz.
    ///
    /// # Errors
    ///
    /// Returns an error when the rate combination is unsupported.
    pub fn new(input_rate: u32, output_rate: u32) -> Result<Self, ResampleError> {
        let inner = if input_rate == output_rate {
            None
        } else {
            Some(FastFixedIn::<f32>::new(
                f64::from(output_rate) / f64::from(input_rate),
                1.0,
                PolynomialDegree::Septic,
                CHUNK_SIZE,
                1,
            )?)
        };
        Ok(Self {
            inner,
            pending: Vec::with_capacity(CHUNK_SIZE * 2),
        })
    }

    /// Feeds input samples and returns whatever output is ready.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying resampler fails.
    pub fn process(&mut self, input: &[f32]) -> Result<Vec<f32>, ResampleError> {
        let Some(inner) = &mut self.inner else {
            return Ok(input.to_vec());
        };
        self.pending.extend_from_slice(input);
        let mut output = Vec::new();
        while self.pending.len() >= CHUNK_SIZE {
            let chunk: Vec<f32> = self.pending.drain(..CHUNK_SIZE).collect();
            let mut result = inner.process(&[chunk], None)?;
            output.append(&mut result[0]);
        }
        Ok(output)
    }

    /// Flushes buffered samples, padding the final partial chunk.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying resampler fails.
    pub fn flush(&mut self) -> Result<Vec<f32>, ResampleError> {
        let Some(inner) = &mut self.inner else {
            return Ok(Vec::new());
        };
        if self.pending.is_empty() {
            return Ok(Vec::new());
        }
        let remainder: Vec<f32> = self.pending.drain(..).collect();
        let mut result = inner.process_partial(Some(&[remainder]), None)?;
        Ok(std::mem::take(&mut result[0]))
    }
}

#[cfg(test)]
mod tests {
    use super::MonoResampler;

    fn sine(rate: u32, frequency: f32, seconds: f32) -> Vec<f32> {
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_sign_loss,
            clippy::cast_possible_truncation
        )]
        let count = (rate as f32 * seconds) as usize;
        (0..count)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32 / rate as f32;
                (t * frequency * 2.0 * std::f32::consts::PI).sin()
            })
            .collect()
    }

    fn zero_crossings(samples: &[f32]) -> usize {
        samples
            .windows(2)
            .filter(|pair| (pair[0] >= 0.0) != (pair[1] >= 0.0))
            .count()
    }

    #[test]
    fn resamples_48k_to_16k_preserving_tone_frequency() {
        let mut resampler = MonoResampler::new(48_000, 16_000).unwrap();
        let input = sine(48_000, 440.0, 1.0);
        let mut output = Vec::new();
        for chunk in input.chunks(480) {
            output.extend(resampler.process(chunk).unwrap());
        }
        output.extend(resampler.flush().unwrap());

        // Expect roughly one second of 16 kHz output.
        assert!(
            (15_000..=17_000).contains(&output.len()),
            "unexpected output length {}",
            output.len()
        );
        // 440 Hz sine has ~880 zero crossings per second.
        let crossings = zero_crossings(&output);
        assert!(
            (800..=960).contains(&crossings),
            "unexpected crossing count {crossings}"
        );
    }

    #[test]
    fn passthrough_when_rates_match() {
        let mut resampler = MonoResampler::new(16_000, 16_000).unwrap();
        let input = sine(16_000, 100.0, 0.1);
        let output = resampler.process(&input).unwrap();
        assert_eq!(output.len(), input.len());
    }
}
