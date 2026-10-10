//! Sound effects synthesized at startup, so the game needs no asset files.

use std::f32::consts::TAU;

const RATE: u32 = 44100;

/// The rail: a heavy boom with a sub-bass kick under it, a saturated zap tearing down from a scream to a growl, a
/// metallic ring and an electric sizzle trailing off. Soft-clipped hard, so it hits loud.
pub fn rail() -> Vec<u8> {
    let (mut noise, mut body, mut fizz) = (Noise::new(7), 0.0f32, 0.0f32);
    let (mut kick, mut zap, mut buzz) = (0.0f32, 0.0f32, 0.0f32);
    let mut ring = [0.0f32; 3];
    render(1.2, move |t| {
        let step = |phase: &mut f32, hz: f32| *phase = (*phase + hz / RATE as f32).fract();
        // A kick drum's pitch drop, 160 down to 42 Hz.
        step(&mut kick, 42.0 + 118.0 * (-t * 28.0).exp());
        let kick = (kick * TAU).sin() * (t * 2000.0).min(1.0) * (-t * 9.0).exp() * 1.4;
        // Low-passed noise: a blast, not a hiss.
        let n = noise.next();
        body += (n - body) * 0.18;
        let blast = body * 3.0 * (-t * 22.0).exp();
        let crack = n * (-t * 90.0).exp() * 0.5;
        // A sawtooth falling from 2.2 kHz to 70 Hz, saturated into a growl.
        step(&mut zap, 70.0 + 2100.0 * (-t * 16.0).exp());
        let zap = ((zap * 2.0 - 1.0) * 3.0).tanh() * (-t * 5.5).exp() * 0.55;
        // Inharmonic partials, like struck metal.
        let mut metal = 0.0;
        for (phase, (hz, k)) in ring.iter_mut().zip([(523.0, 0.5), (1397.0, 0.3), (2213.0, 0.2)]) {
            step(phase, hz);
            metal += (*phase * TAU).sin() * k;
        }
        let metal = metal * (-t * 4.5).exp() * 0.25;
        // Mains-hum square through crackling noise, dying slowly.
        step(&mut buzz, 98.0);
        fizz += (noise.next() - fizz) * 0.6;
        let sizzle = (buzz * TAU).sin().signum() * (0.5 + fizz.abs()) * (-t * 3.2).exp() * 0.16;
        ((kick + blast + crack + zap + metal + sizzle) * 1.6).tanh() * 1.9
    })
}

/// You fragged someone: a bright two-note ding.
pub fn frag() -> Vec<u8> {
    synth(0.3, |t| if t < 0.07 { 1320.0 } else { 1980.0 }, |t| (-t * 12.0).exp() * 0.8, |p| (p * TAU).sin())
}

/// Someone burst into gibs: a wet low thump.
pub fn gib() -> Vec<u8> {
    let (mut noise, mut low) = (Noise::new(3), 0.0f32);
    render(0.5, move |t| {
        low += (noise.next() - low) * 0.05;
        low * 6.0 * (-t * 9.0).exp() + (t * 70.0 * TAU).sin() * (-t * 14.0).exp()
    })
}

/// Jump pad: a rising whoosh.
pub fn pad() -> Vec<u8> {
    let mut noise = Noise::new(5);
    synth(0.5, |t| 200.0 + 900.0 * t, |t| (t * 12.0).min(1.0) * (1.0 - t / 0.5), move |p| (p * TAU).sin() * 0.6 + noise.next() * 0.25)
}

pub fn jump() -> Vec<u8> {
    synth(0.12, |t| 320.0 + 500.0 * t, |t| 0.35 * (1.0 - t / 0.12), |p| (p * TAU).sin())
}

/// Back in: a rising arpeggio.
pub fn respawn() -> Vec<u8> {
    synth(0.45, |t| [523.0, 659.0, 784.0, 1046.0][((t / 0.1) as usize).min(3)], |t| 0.45 * (1.0 - t / 0.45), |p| (p * TAU).sin())
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

