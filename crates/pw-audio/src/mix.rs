//! Realtime-safe helpers used inside the audio callback.
//!
//! These functions must not allocate, lock or log: they run inside
//! the high-priority cpal callback.

/// Averages interleaved frames into the mono scratch buffer.
/// Returns the number of mono samples written (truncates when the
/// scratch buffer is smaller than the input frame count).
pub fn write_interleaved_as_mono(
    interleaved: &[f32],
    channels: usize,
    mono_scratch: &mut [f32],
) -> usize {
    if channels == 0 {
        return 0;
    }
    let frames = (interleaved.len() / channels).min(mono_scratch.len());
    if channels == 1 {
        mono_scratch[..frames].copy_from_slice(&interleaved[..frames]);
        return frames;
    }
    #[allow(clippy::cast_precision_loss)]
    let scale = 1.0 / channels as f32;
    for (frame_index, scratch) in mono_scratch.iter_mut().take(frames).enumerate() {
        let start = frame_index * channels;
        let mut sum = 0.0_f32;
        for sample in &interleaved[start..start + channels] {
            sum += *sample;
        }
        *scratch = sum * scale;
    }
    frames
}

/// Pushes samples into the bounded ring buffer, returning how many
/// were dropped because the buffer was full.
pub fn push_mono_counting_drops(producer: &mut rtrb::Producer<f32>, samples: &[f32]) -> usize {
    let mut dropped = 0;
    for sample in samples {
        if producer.push(*sample).is_err() {
            dropped += 1;
        }
    }
    dropped
}

#[cfg(test)]
mod tests {
    use super::{push_mono_counting_drops, write_interleaved_as_mono};

    #[test]
    fn averages_interleaved_channels_into_mono() {
        let interleaved = [0.2_f32, 0.4, -1.0, 1.0];
        let mut mono = [0.0_f32; 2];
        let written = write_interleaved_as_mono(&interleaved, 2, &mut mono);
        assert_eq!(written, 2);
        assert!((mono[0] - 0.3).abs() < 1e-6);
        assert!(mono[1].abs() < 1e-6);
    }

    #[test]
    fn passes_mono_input_through() {
        let input = [0.5_f32, -0.5];
        let mut mono = [0.0_f32; 2];
        let written = write_interleaved_as_mono(&input, 1, &mut mono);
        assert_eq!(written, 2);
        assert_eq!(&mono[..2], &input);
    }

    #[test]
    fn truncates_when_the_scratch_buffer_is_small() {
        let interleaved = [0.1_f32; 8];
        let mut mono = [0.0_f32; 2];
        let written = write_interleaved_as_mono(&interleaved, 2, &mut mono);
        assert_eq!(written, 2);
    }

    #[test]
    fn counts_dropped_samples_when_the_ring_buffer_is_full() {
        let (mut producer, mut consumer) = rtrb::RingBuffer::<f32>::new(4);
        assert_eq!(push_mono_counting_drops(&mut producer, &[0.0; 3]), 0);
        // one slot remains: two of the next three samples are dropped.
        assert_eq!(push_mono_counting_drops(&mut producer, &[0.0; 3]), 2);
        let mut read = 0;
        while consumer.pop().is_ok() {
            read += 1;
        }
        assert_eq!(read, 4);
    }
}
