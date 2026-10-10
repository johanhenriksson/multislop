//! Sound effects synthesized at startup, so the game needs no asset files. The launcher, explosion, reload, hit and
//! hurt sounds are dust's; the stadium (crowd, horn, whistle) and the ball are new.

use std::f32::consts::TAU;

const RATE: u32 = 44100;

/// A deep whoomp as the rocket leaves, rolling off the stands.
pub fn launch() -> Vec<u8> {
    let mut rng = 0xC0FF_EE11u32;
    let mut s = shot(&mut rng, 0.9, 0.2, 42.0, 1.2).echo(0.2, 0.35, 2);
    s.normalize(1.0);
    wav(&s.0)
}

/// Low-passed noise with a long tail.
pub fn explosion() -> Vec<u8> {
    let (mut rng, mut low) = (0x1234_5678u32, 0.0f32);
    synth(1.6, |_| 0.0, |t| (1.0 - t / 1.6).powi(2) * 2.0, move |_| {
        low += (noise(&mut rng) - low) * 0.08;
        low * 3.0
    })
}

/// Magazine out, magazine in: here, the tube unloaded and a fresh load slid home.
pub fn reload() -> Vec<u8> {
    let mut rng = 0x5EEDu32;
    render(0.5, move |t| {
        let click = |at: f32| if t > at { (-(t - at) * 120.0).exp() } else { 0.0 };
        noise(&mut rng) * (click(0.0) + click(0.32) * 1.3) * 0.8
    })
}

/// You killed someone: a bright two-note ding.
pub fn hit() -> Vec<u8> {
    synth(0.25, |t| if t < 0.06 { 1320.0 } else { 1760.0 }, |t| (-t * 14.0).exp(), |p| (p * TAU).sin())
}

/// Dull thud when you take damage.
pub fn hurt() -> Vec<u8> {
    let (mut rng, mut low) = (0xD00Du32, 0.0f32);
    render(0.2, move |t| {
        low += (noise(&mut rng) - low) * 0.06;
        ((t * 110.0 * TAU).sin() * 0.7 + low * 3.0) * (-t * 18.0).exp()
    })
}

/// The ball run into: a deep, round thump, like kicking a huge inflated ball.
pub fn thump() -> Vec<u8> {
    let (mut rng, mut low) = (0x7A11u32, 0.0f32);
    let mut phase = 0.0f32;
    render(0.45, move |t| {
        phase = (phase + (55.0 + 70.0 * (-t * 25.0).exp()) / RATE as f32).fract();
        low += (noise(&mut rng) - low) * 0.04;
        (phase * TAU).sin() * (-t * 9.0).exp() * 1.4 + low * 4.0 * (-t * 40.0).exp()
    })
}

/// The ball blasted: a hard metallic clang, inharmonic partials ringing out over a thump.
pub fn clang() -> Vec<u8> {
    let mut rng = 0xC1A6u32;
    render(1.0, move |t| {
        let ring: f32 = [(410.0, 1.0, 5.0), (1093.0, 0.6, 7.0), (1781.0, 0.4, 9.0), (2617.0, 0.25, 12.0)]
            .iter()
            .map(|&(hz, amp, decay)| (t * hz * TAU).sin() * amp * (-t * decay).exp())
            .sum();
        let thud = (t * 70.0 * TAU).sin() * (-t * 14.0).exp();
        ring * 0.55 + thud * 0.8 + noise(&mut rng) * (-t * 90.0).exp() * 0.6
    })
}

/// The stadium air horn: a brassy chord, swelling in and held for two seconds.
pub fn horn() -> Vec<u8> {
    let mut phases = [0.0f32; 3];
    let mut low = 0.0f32;
    render(2.4, move |t| {
        let wobble = 1.0 + 0.004 * (t * 5.5 * TAU).sin();
        let mut v = 0.0;
        for (p, hz) in phases.iter_mut().zip([233.0, 293.0, 349.0]) {
            *p = (*p + hz * wobble / RATE as f32).fract();
            v += 2.0 * *p - 1.0;
        }
        // Soften the saws a little so it's brassy, not buzzy.
        low += (v - low) * 0.25;
        let env = (t / 0.08).min(1.0) * if t > 2.0 { 1.0 - (t - 2.0) / 0.4 } else { 1.0 };
        (low * 0.5).tanh() * env * 1.2
    })
}

/// The crowd going up: band-passed noise of thousands of voices, swelling in over `rise` seconds and dying away.
fn crowd(seed: u32, secs: f32, rise: f32, loud: f32) -> Vec<u8> {
    let (mut rng, mut low, mut lower, mut flutter) = (seed, 0.0f32, 0.0f32, 0.0f32);
    render(secs, move |t| {
        let n = noise(&mut rng);
        low += (n - low) * 0.18;
        lower += (low - lower) * 0.02;
        // Slow random shimmer, so it sounds like many people rather than a hiss.
        flutter += (noise(&mut rng).abs() - flutter) * 0.0008;
        let env = (t / rise).min(1.0).powf(1.5) * (1.0 - ((t - rise) / (secs - rise)).max(0.0)).powi(2);
        (low - lower) * env * (0.7 + flutter) * loud
    })
}

/// A goal: the roar of the crowd.
pub fn roar() -> Vec<u8> {
    crowd(0xFA75, 4.0, 0.35, 7.0)
}

/// Play near a goal: the crowd rising in anticipation.
pub fn swell() -> Vec<u8> {
    crowd(0x0001_5E11, 2.6, 1.1, 4.0)
}

/// The crowd murmuring under everything, 8 s and seamless when looped: its swells come in whole cycles.
pub fn ambience() -> Vec<u8> {
    let (mut rng, mut low, mut lower) = (0xA4B1u32, 0.0f32, 0.0f32);
    render(8.0, move |t| {
        low += (noise(&mut rng) - low) * 0.12;
        lower += (low - lower) * 0.015;
        let swell = 0.75 + 0.15 * (t * TAU / 8.0).sin() + 0.1 * (t * TAU * 3.0 / 8.0).sin();
        (low - lower) * swell * 2.2
    })
}

/// Health: a rising bell chime.
pub fn chime() -> Vec<u8> {
    render(0.5, |t| {
        let note = |at: f32, hz: f32| if t > at { ((t - at) * hz * TAU).sin() * (-(t - at) * 7.0).exp() } else { 0.0 };
        (note(0.0, 988.0) + note(0.09, 1319.0) + note(0.09, 2638.0) * 0.2) * 0.6
    })
}

/// Armor: plates clanking onto a vest.
pub fn clank() -> Vec<u8> {
    let mut rng = 0xA77Fu32;
    render(0.45, move |t| {
        let hit = |at: f32| if t > at { let u = t - at; ((u * 620.0 * TAU).sin() + (u * 1530.0 * TAU).sin() * 0.5) * (-u * 22.0).exp() } else { 0.0 };
        hit(0.0) + hit(0.11) * 0.7 + noise(&mut rng) * (-t * 70.0).exp() * 0.5
    })
}

/// Rockets: a crate lid and two rounds clacking in.
pub fn clack() -> Vec<u8> {
    let mut rng = 0xC1ACu32;
    render(0.4, move |t| {
        let click = |at: f32| if t > at { (-(t - at) * 150.0).exp() } else { 0.0 };
        let knock = (t * 180.0 * TAU).sin() * (-t * 30.0).exp();
        noise(&mut rng) * (click(0.0) + click(0.14) * 0.9 + click(0.24) * 0.9) * 0.9 + knock * 0.5
    })
}

/// Kickoff countdown: a short beep, and a longer, higher one for go.
pub fn beep(go: bool) -> Vec<u8> {
    let (hz, secs) = if go { (1760.0, 0.35) } else { (880.0, 0.15) };
    synth(secs, move |_| hz, move |t| (1.0 - t / secs) * 0.6, |p| (p * TAU).sin() + (p * 2.0 * TAU).sin() * 0.2)
}

/// A referee's pea whistle: a shrill tone trilling as the pea rattles, `blasts` times.
pub fn whistle(blasts: u32) -> Vec<u8> {
    let (mut phase, mut rng) = (0.0f32, 0x7157u32);
    let blast = if blasts > 1 { 0.32 } else { 0.7 };
    render(blasts as f32 * (blast + 0.12), move |t| {
        let local = t % (blast + 0.12);
        if local > blast {
            return 0.0;
        }
        let trill = (t * 32.0 * TAU).sin();
        phase = (phase + (2900.0 + 120.0 * trill) / RATE as f32).fract();
        let env = (local / 0.02).min(1.0) * (1.0 - (local - blast + 0.05).max(0.0) / 0.05);
        ((phase * TAU).sin() * (0.75 + 0.25 * trill) + noise(&mut rng) * 0.08) * env * 0.7
    })
}

/// Jump pad: a rising whoosh.
pub fn pad() -> Vec<u8> {
    let mut rng = 5u32;
    synth(0.5, |t| 200.0 + 900.0 * t, |t| (t * 12.0).min(1.0) * (1.0 - t / 0.5), move |p| (p * TAU).sin() * 0.6 + noise(&mut rng) * 0.25)
}

pub fn jump() -> Vec<u8> {
    synth(0.12, |t| 320.0 + 500.0 * t, |t| 0.35 * (1.0 - t / 0.12), |p| (p * TAU).sin())
}

/// Back in: a rising arpeggio.
pub fn respawn() -> Vec<u8> {
    synth(0.45, |t| [523.0, 659.0, 784.0, 1046.0][((t / 0.1) as usize).min(3)], |t| 0.45 * (1.0 - t / 0.45), |p| (p * TAU).sin())
}

/// Samples at RATE, with a few layering helpers.
struct Samples(Vec<f32>);

impl Samples {
    /// Adds `other` starting `at` seconds in, scaled by `gain`, growing as needed.
    fn mix(mut self, other: Samples, at: f32, gain: f32) -> Samples {
        let start = (at * RATE as f32) as usize;
        if self.0.len() < start + other.0.len() {
            self.0.resize(start + other.0.len(), 0.0);
        }
        for (i, v) in other.0.into_iter().enumerate() {
            self.0[start + i] += v * gain;
        }
        self
    }

    /// Repeats the sound `count` times, each `delay` seconds later and `gain` times quieter.
    fn echo(self, delay: f32, gain: f32, count: u32) -> Samples {
        let dry = self.0.clone();
        (1..=count).fold(self, |s, n| s.mix(Samples(dry.clone()), delay * n as f32, gain.powi(n as i32)))
    }

    fn normalize(&mut self, peak: f32) {
        let max = self.0.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
        self.0.iter_mut().for_each(|v| *v *= peak / max * 2.0);
    }
}

fn samples(secs: f32, mut f: impl FnMut(f32) -> f32) -> Samples {
    Samples((0..(secs * RATE as f32) as u32).map(|i| f(i as f32 / RATE as f32)).collect())
}

/// The report: low-passed noise crack (brightness 0..1) plus a falling sine thump.
fn shot(rng: &mut u32, secs: f32, bright: f32, thump: f32, loud: f32) -> Samples {
    let (mut low, mut phase, mut seed) = (0.0f32, 0.0f32, *rng);
    *rng = rng.wrapping_mul(747796405).wrapping_add(1);
    samples(secs, move |t| {
        let crack = (-t * 38.0 / secs).exp();
        let tail = (-t * 6.0 / secs).exp() * 0.35;
        low += (noise(&mut seed) - low) * bright;
        phase = (phase + thump * (1.0 + 2.0 * (-t * 30.0).exp()) / RATE as f32).fract();
        let boom = (phase * TAU).sin() * (-t * 9.0 / secs).exp();
        (low * 2.2 * (crack + tail) + boom * 0.9) * loud
    })
}

fn noise(rng: &mut u32) -> f32 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 17;
    *rng ^= *rng << 5;
    *rng as f32 / u32::MAX as f32 * 2.0 - 1.0
}

/// Renders a mono 16-bit WAV. `freq` and `amp` are functions of time; frequency is integrated so sweeps stay smooth.
fn synth(secs: f32, freq: impl Fn(f32) -> f32, amp: impl Fn(f32) -> f32, mut wave: impl FnMut(f32) -> f32) -> Vec<u8> {
    let mut phase = 0.0f32;
    render(secs, move |t| {
        phase = (phase + freq(t) / RATE as f32).fract();
        wave(phase) * amp(t)
    })
}

/// Renders a mono 16-bit WAV from a sample function of time.
fn render(secs: f32, sample: impl FnMut(f32) -> f32) -> Vec<u8> {
    wav(&samples(secs, sample).0)
}

fn wav(samples: &[f32]) -> Vec<u8> {
    let n = samples.len() as u32;
    let mut out = Vec::with_capacity(44 + n as usize * 2);
    for chunk in [&b"RIFF"[..], &(36 + n * 2).to_le_bytes(), b"WAVEfmt ", &16u32.to_le_bytes(), &1u16.to_le_bytes(),
        &1u16.to_le_bytes(), &RATE.to_le_bytes(), &(RATE * 2).to_le_bytes(), &2u16.to_le_bytes(), &16u16.to_le_bytes(),
        b"data", &(n * 2).to_le_bytes()]
    {
        out.extend_from_slice(chunk);
    }
    for &v in samples {
        let s = (v * 0.5).clamp(-1.0, 1.0);
        out.extend_from_slice(&((s * i16::MAX as f32) as i16).to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    /// Every sound is a well-formed WAV; `FPS16_DUMP_SFX=dir` writes them out for listening.
    #[test]
    fn valid_wavs() {
        use super::*;
        let all = [
            ("launch", launch()), ("explosion", explosion()), ("thump", thump()), ("clang", clang()), ("horn", horn()),
            ("roar", roar()), ("swell", swell()), ("ambience", ambience()), ("chime", chime()), ("clank", clank()),
            ("clack", clack()), ("beep", beep(false)), ("go", beep(true)), ("whistle", whistle(1)), ("end", whistle(3)),
        ];
        for (name, w) in &all {
            assert_eq!(&w[..4], b"RIFF", "{name}");
            assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize, w.len() - 44, "{name}");
        }
        if let Some(dir) = std::env::var_os("FPS16_DUMP_SFX") {
            for (name, w) in all {
                std::fs::write(std::path::Path::new(&dir).join(format!("{name}.wav")), w).unwrap();
            }
        }
    }
}
