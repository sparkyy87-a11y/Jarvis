//! Umrechnung beliebiger Mikrofon-Abtastraten auf 16 kHz mono.

/// Mehrkanal-Audio (verschachtelt) zu mono mitteln.
pub fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|c| c.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Streaming-Resampler: Tiefpass (gefensterter Sinc, gegen Aliasing) und
/// lineare Interpolation. Für Sprache genau genug und sehr billig.
pub struct Resampler {
    step: f64,
    taps: Vec<f32>,
    history: Vec<f32>,
    filtered: Vec<f32>,
    pos: f64,
}

impl Resampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        const N: usize = 31;
        let cutoff = (0.45 * out_rate as f64 / in_rate as f64).min(0.45);
        let mid = (N / 2) as f64;
        let mut taps: Vec<f32> = (0..N)
            .map(|i| {
                let x = i as f64 - mid;
                let sinc = if x == 0.0 {
                    2.0 * cutoff
                } else {
                    (2.0 * std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x)
                };
                let hamming =
                    0.54 - 0.46 * (2.0 * std::f64::consts::PI * i as f64 / (N - 1) as f64).cos();
                (sinc * hamming) as f32
            })
            .collect();
        let sum: f32 = taps.iter().sum();
        taps.iter_mut().for_each(|t| *t /= sum);
        Self {
            step: in_rate as f64 / out_rate as f64,
            history: vec![0.0; N - 1],
            taps,
            filtered: Vec::new(),
            pos: 0.0,
        }
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if self.step == 1.0 {
            return input.to_vec();
        }
        let mut ext = std::mem::take(&mut self.history);
        ext.extend_from_slice(input);
        let n = self.taps.len();
        for i in 0..input.len() {
            let window = &ext[i..i + n];
            self.filtered
                .push(window.iter().zip(&self.taps).map(|(a, b)| a * b).sum());
        }
        self.history = ext[ext.len() - (n - 1)..].to_vec();

        let mut out = Vec::with_capacity((input.len() as f64 / self.step) as usize + 1);
        while self.pos + 1.0 < self.filtered.len() as f64 {
            let i = self.pos as usize;
            let frac = (self.pos - i as f64) as f32;
            out.push(self.filtered[i] * (1.0 - frac) + self.filtered[i + 1] * frac);
            self.pos += self.step;
        }
        let consumed = (self.pos as usize).min(self.filtered.len());
        self.filtered.drain(..consumed);
        self.pos -= consumed as f64;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, secs: f32) -> Vec<f32> {
        (0..(rate as f32 * secs) as usize)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin())
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn length_and_streaming() {
        let input = sine(440.0, 48_000, 1.0);
        let mut r = Resampler::new(48_000, 16_000);
        // In unregelmäßigen Häppchen, wie vom Mikrofon.
        let out: Vec<f32> = input.chunks(997).flat_map(|c| r.process(c)).collect();
        assert!((out.len() as i64 - 16_000).abs() < 40, "{}", out.len());
        // Sprachfrequenzen bleiben erhalten (Sinus: RMS ≈ 0,707).
        assert!((rms(&out[1000..]) - 0.707).abs() < 0.05);
    }

    #[test]
    fn removes_frequencies_above_8khz() {
        let mut r = Resampler::new(44_100, 16_000);
        let out = r.process(&sine(12_000.0, 44_100, 0.5));
        assert!(rms(&out[500..]) < 0.1, "Aliasing: {}", rms(&out[500..]));
    }

    #[test]
    fn downmix_stereo() {
        assert_eq!(downmix(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
    }
}
