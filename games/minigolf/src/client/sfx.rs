//! Sound effects synthesized at startup, so the game needs no asset files.

use std::f32::consts::TAU;

const RATE: u32 = 44100;

/// The putter on the ball: a short woody knock.
pub fn tock() -> Vec<u8> {
    let mut noise = Noise::new(11);
    synth(0.12, |t| 900.0 - 2000.0 * t, |t| (-t * 60.0).exp(), move |p| (p * TAU).sin() * 0.8 + noise.next() * 0.3)
}

/// Off a rail: a bright clink.
pub fn clink() -> Vec<u8> {
    let (mut a, mut b) = (0.0f32, 0.0f32);
    render(0.25, move |t| {
        a = (a + 2400.0 / RATE as f32).fract();
        b = (b + 3570.0 / RATE as f32).fract();
        ((a * TAU).sin() + (b * TAU).sin() * 0.5) * (-t * 28.0).exp() * 0.7
    })
}

/// Into the cup: a few quick knocks rattling down, and a hollow thunk.
pub fn rattle() -> Vec<u8> {
    let mut noise = Noise::new(3);
    render(0.45, move |t| {
        let knock = [0.0, 0.07, 0.12, 0.155, 0.18].iter().map(|&at| if t >= at { (-(t - at) * 90.0).exp() } else { 0.0 }).sum::<f32>();
        let thunk = if t > 0.2 { ((t - 0.2) * 180.0 * TAU).sin() * (-(t - 0.2) * 14.0).exp() } else { 0.0 };
        (noise.next() * 0.5 + (t * 1800.0 * TAU).sin() * 0.5) * knock * 0.8 + thunk
    })
}

/// Into the water: a plop and a bubbling wash.
pub fn splash() -> Vec<u8> {
    let (mut noise, mut low) = (Noise::new(5), 0.0f32);
    render(0.7, move |t| {
        let n = noise.next();
        low += (n - low) * 0.08;
        low * 4.0 * (-t * 6.0).exp() + ((300.0 - 400.0 * t) * t * TAU).sin() * (-t * 18.0).exp()
    })
}

/// Off into the void: a falling whistle.
pub fn whoosh() -> Vec<u8> {
    synth(0.8, |t| 1200.0 - 1100.0 * t, |t| (t * 10.0).min(1.0) * (1.0 - t / 0.8) * 0.4, |p| (p * TAU).sin())
}

/// Under par: a short ripple of applause.
pub fn cheer() -> Vec<u8> {
    let (mut noise, mut claps) = (Noise::new(9), Noise::new(17));
    let mut env = 0.0f32;
    render(1.6, move |t| {
        // Random claps, thinning out.
        if claps.next() > 1.0 - 0.0016 * (1.0 - t / 1.6) {
            env = 1.0;
        }
        env *= 0.996;
        noise.next() * env * 0.7
    })
}

/// The start of a hole: a two-note chime.
pub fn go() -> Vec<u8> {
    synth(0.5, |t| if t < 0.12 { 784.0 } else { 1175.0 }, |t| 0.45 * (1.0 - t / 0.5), |p| (p * TAU).sin())
}

/// A soft wind under everything.
pub fn ambience() -> Vec<u8> {
    let (mut noise, mut low) = (Noise::new(21), 0.0f32);
    render(6.0, move |t| {
        low += (noise.next() - low) * 0.01;
        // Fades in and out over the loop so its ends meet.
        low * 2.5 * (0.6 + 0.4 * (t / 6.0 * TAU).sin().abs())
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
