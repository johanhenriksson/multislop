//! Sound effects synthesized at startup, so the game needs no asset files.

use std::f32::consts::TAU;

const RATE: u32 = 44100;

/// Lasered: a sharp crack and a falling zap.
pub fn zap() -> Vec<u8> {
    let mut noise = Noise::new(7);
    let mut phase = 0.0f32;
    render(0.5, move |t| {
        phase = (phase + (3200.0 * (-t * 9.0).exp() + 180.0) / RATE as f32).fract();
        noise.next() * (-t * 60.0).exp() + (phase * TAU).sin() * (-t * 7.0).exp() * 0.6
    })
}

/// Into the lava: a hissing sizzle over a low plop.
pub fn splash() -> Vec<u8> {
    let (mut noise, mut low) = (Noise::new(3), 0.0f32);
    render(0.8, move |t| {
        let n = noise.next();
        low += (n - low) * 0.05;
        low * 5.0 * (-t * 9.0).exp() + (t * 90.0 * TAU).sin() * (-t * 14.0).exp() + n * 0.3 * (-t * 3.0).exp()
    })
}

/// A checkpoint: a bright two-note ding.
pub fn checkpoint() -> Vec<u8> {
    synth(0.3, |t| if t < 0.07 { 1320.0 } else { 1980.0 }, |t| (-t * 12.0).exp() * 0.8, |p| (p * TAU).sin())
}

/// Over the line: a rising arpeggio that rings out.
pub fn finish() -> Vec<u8> {
    synth(0.9, |t| [523.0, 659.0, 784.0, 1046.0, 1318.0][((t / 0.09) as usize).min(4)], |t| 0.45 * (1.0 - t / 0.9), |p| (p * TAU).sin())
}

/// The countdown: a short beep, and a higher, longer one for go.
pub fn beep(go: bool) -> Vec<u8> {
    let (secs, freq) = if go { (0.5, 880.0) } else { (0.18, 440.0) };
    synth(secs, move |_| freq, move |t| 0.4 * (1.0 - t / secs), |p| if p < 0.5 { 0.6 } else { -0.6 })
}

/// Jump pad: a rising whoosh.
pub fn pad() -> Vec<u8> {
    let mut noise = Noise::new(5);
    synth(0.5, |t| 200.0 + 900.0 * t, |t| (t * 12.0).min(1.0) * (1.0 - t / 0.5), move |p| (p * TAU).sin() * 0.6 + noise.next() * 0.25)
}

pub fn jump() -> Vec<u8> {
    synth(0.12, |t| 320.0 + 500.0 * t, |t| 0.35 * (1.0 - t / 0.12), |p| (p * TAU).sin())
}

/// Touching down: a short, low thud.
pub fn land() -> Vec<u8> {
    synth(0.12, |t| 150.0 - 500.0 * t, |t| 0.6 * (-t * 35.0).exp(), |p| (p * TAU).sin())
}

/// A deep sci-fi drone under everything.
pub fn ambience() -> Vec<u8> {
    let (mut a, mut b) = (0.0f32, 0.0f32);
    // Whole cycles of both tones in 4 s, so the loop is seamless.
    render(4.0, move |t| {
        a = (a + 55.0 / RATE as f32).fract();
        b = (b + 82.75 / RATE as f32).fract();
        ((a * TAU).sin() + (b * TAU).sin() * 0.6) * (0.18 + 0.06 * (t * TAU * 0.25).sin())
    })
}

struct Noise(u32);

impl Noise {
    fn new(seed: u32) -> Noise {
        Noise(seed.wrapping_mul(0x9E37_79B9) | 1)
    }

    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

fn synth(secs: f32, freq: impl Fn(f32) -> f32, amp: impl Fn(f32) -> f32, mut wave: impl FnMut(f32) -> f32) -> Vec<u8> {
    let mut phase = 0.0f32;
    render(secs, move |t| {
        phase = (phase + freq(t) / RATE as f32).fract();
        wave(phase) * amp(t)
    })
}

/// A mono 16-bit WAV from a sample function of time.
fn render(secs: f32, mut sample: impl FnMut(f32) -> f32) -> Vec<u8> {
    let n = (secs * RATE as f32) as u32;
    let mut out = Vec::with_capacity(44 + n as usize * 2);
    for chunk in [&b"RIFF"[..], &(36 + n * 2).to_le_bytes(), b"WAVEfmt ", &16u32.to_le_bytes(), &1u16.to_le_bytes(),
        &1u16.to_le_bytes(), &RATE.to_le_bytes(), &(RATE * 2).to_le_bytes(), &2u16.to_le_bytes(), &16u16.to_le_bytes(),
        b"data", &(n * 2).to_le_bytes()]
    {
        out.extend_from_slice(chunk);
    }
    for i in 0..n {
        let s = (sample(i as f32 / RATE as f32) * 0.5).clamp(-1.0, 1.0);
        out.extend_from_slice(&((s * i16::MAX as f32) as i16).to_le_bytes());
    }
    out
}
