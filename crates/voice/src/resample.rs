//! Convert captured audio to what whisper.cpp expects: **16 kHz mono f32**.
//!
//! Mic capture comes in at the device's native rate (commonly 44.1/48 kHz)
//! and channel count. We downmix to mono and linear-resample to 16 kHz.
//! Linear interpolation is plenty for speech recognition; a sinc resampler
//! (`rubato`) is a future quality upgrade.

/// Whisper's required sample rate.
pub const WHISPER_SAMPLE_RATE: u32 = 16_000;

/// Downmix interleaved `channels`-channel audio to mono, then resample to 16 kHz.
pub fn to_whisper_mono_16k(samples: &[f32], sample_rate: u32, channels: u16) -> Vec<f32> {
    let mono = downmix(samples, channels);
    resample_linear(&mono, sample_rate, WHISPER_SAMPLE_RATE)
}

/// Average interleaved frames down to a single channel.
pub fn downmix(samples: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    let ch = channels as usize;
    samples
        .chunks(ch)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

/// Linear-interpolation resampler for mono audio.
pub fn resample_linear(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if input.is_empty() || from == to {
        return input.to_vec();
    }
    let ratio = to as f64 / from as f64;
    let out_len = ((input.len() as f64) * ratio).round() as usize;
    let last = input.len() - 1;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f64 / ratio;
        let idx = src.floor() as usize;
        let frac = (src - idx as f64) as f32;
        let a = input[idx.min(last)];
        let b = input[(idx + 1).min(last)];
        out.push(a + (b - a) * frac);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_stereo_averages_pairs() {
        // L/R interleaved: (1,0),(0,1),(0.5,0.5) → 0.5, 0.5, 0.5
        let mono = downmix(&[1.0, 0.0, 0.0, 1.0, 0.5, 0.5], 2);
        assert_eq!(mono, vec![0.5, 0.5, 0.5]);
    }

    #[test]
    fn downmix_mono_is_identity() {
        let s = [0.1, 0.2, 0.3];
        assert_eq!(downmix(&s, 1), s.to_vec());
    }

    #[test]
    fn resample_same_rate_is_identity() {
        let s = vec![0.0, 0.5, 1.0, -1.0];
        assert_eq!(resample_linear(&s, 16_000, 16_000), s);
    }

    #[test]
    fn resample_48k_to_16k_thirds_the_length() {
        let input = vec![0.0_f32; 4800]; // 0.1s at 48 kHz
        let out = resample_linear(&input, 48_000, 16_000);
        // 0.1s at 16 kHz ≈ 1600 samples.
        assert_eq!(out.len(), 1600);
    }

    #[test]
    fn to_whisper_combines_downmix_and_resample() {
        // 2 channels at 32 kHz → mono at 16 kHz halves the frame count.
        let frames = 320; // interleaved stereo frames
        let input = vec![0.25_f32; frames * 2];
        let out = to_whisper_mono_16k(&input, 32_000, 2);
        assert_eq!(out.len(), frames / 2);
        assert!(out.iter().all(|&x| (x - 0.25).abs() < 1e-6));
    }
}
