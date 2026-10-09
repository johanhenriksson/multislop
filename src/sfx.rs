//! Sound effects synthesized at startup, so the game stays a single binary with no asset files.

use crate::weapons::Weapon;
use std::f32::consts::TAU;

const RATE: u32 = 44100;

pub fn hit() -> Vec<u8> {
    synth(0.25, |t| if t < 0.06 { 1320.0 } else { 1760.0 }, |t| (-t * 14.0).exp(), |p| (p * TAU).sin())
}

pub fn beep() -> Vec<u8> {
    synth(0.09, |_| 2400.0, |t| 1.0 - t / 0.09, |p| (p * TAU).sin())
}

/// Low-passed noise with a long tail.
pub fn explosion() -> Vec<u8> {
    let (mut rng, mut low) = (0x1234_5678u32, 0.0f32);
    synth(1.6, |_| 0.0, |t| (1.0 - t / 1.6).powi(2) * 2.0, move |_| {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        low += (rng as f32 / u32::MAX as f32 * 2.0 - 1.0 - low) * 0.08;
        low * 3.0
    })
}

pub fn death() -> Vec<u8> {
    synth(0.9, |t| 300.0 * (-t * 2.5).exp() + 40.0, |t| 1.0 - t / 0.9, |p| 2.0 * (p - (p + 0.5).floor()))
}

/// Each weapon has its own character: silenced weapons thwip, the Deagle and AWP boom and echo, the AWP works its
/// bolt afterwards, the AK cracks harsh with a bolt clack, the MP5 and Glock are light and snappy.
pub fn gunshot(weapon: Weapon) -> Vec<u8> {
    let mut rng = 0xC0FF_EE11u32 ^ weapon as u32;
    let mut s = match weapon {
        Weapon::Knife => return knife(),
        // Light, high, short pop with a little slide ping.
        Weapon::Glock => shot(&mut rng, 0.18, 0.6, 240.0, 0.7)
            .mix(tone(0.04, 2600.0, 60.0), 0.0, 0.15)
            .mix(click(&mut rng, 0.03), 0.07, 0.3),
        // Silenced: muffled low thwip and the slide cycling.
        Weapon::Usp => shot(&mut rng, 0.12, 0.08, 150.0, 0.35).mix(click(&mut rng, 0.03), 0.04, 0.5),
        // Hand cannon: deep boom, long tail, echo.
        Weapon::Deagle => shot(&mut rng, 0.6, 0.35, 75.0, 1.0).echo(0.13, 0.35, 2),
        // Light rapid pap with a metallic rattle.
        Weapon::Mp5 => shot(&mut rng, 0.13, 0.7, 260.0, 0.55).mix(click(&mut rng, 0.02), 0.035, 0.25),
        // Harsh, punchy crack, overdriven, with a bolt clack.
        Weapon::Ak47 => shot(&mut rng, 0.4, 0.4, 95.0, 1.3).clip(0.55).mix(click(&mut rng, 0.04), 0.06, 0.35),
        // Silenced rifle: the classic tshk.
        Weapon::M4a1 => shot(&mut rng, 0.16, 0.18, 180.0, 0.5).mix(click(&mut rng, 0.035), 0.03, 0.55),
        // Huge crack and boom rolling off, then the bolt worked back and forth.
        Weapon::Awp => shot(&mut rng, 1.1, 0.45, 60.0, 1.2)
            .echo(0.22, 0.4, 3)
            .mix(click(&mut rng, 0.05), 0.75, 0.5)
            .mix(click(&mut rng, 0.05), 0.95, 0.5),
    };
    let peak = match weapon {
        Weapon::Usp | Weapon::M4a1 => 0.4,
        Weapon::Glock | Weapon::Mp5 => 0.7,
        Weapon::Ak47 => 0.9,
        _ => 1.0,
    };
    s.normalize(peak);
    wav(&s.0)
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

    /// Repeats the sound `count` times, each `delay` seconds later and `gain` times quieter, like a canyon echo.
    fn echo(self, delay: f32, gain: f32, count: u32) -> Samples {
        let dry = self.0.clone();
        (1..=count).fold(self, |s, n| s.mix(Samples(dry.clone()), delay * n as f32, gain.powi(n as i32)))
    }

    /// Soft-clips for grit.
    fn clip(mut self, drive: f32) -> Samples {
        for v in &mut self.0 {
            *v = (*v / drive).tanh() * drive;
        }
        self
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

/// A short mechanical click: high-passed noise burst.
fn click(rng: &mut u32, secs: f32) -> Samples {
    let (mut low, mut seed) = (0.0f32, *rng ^ 0x9E37);
    samples(secs, move |t| {
        let n = noise(&mut seed);
        low += (n - low) * 0.2;
        (n - low) * (-t * 160.0).exp() * 1.5
    })
}

fn tone(secs: f32, hz: f32, decay: f32) -> Samples {
    samples(secs, |t| (t * hz * TAU).sin() * (-t * decay).exp())
}

/// Announcer lines rendered at build time (see build.rs).
pub const VOICE_T_WIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/t_win.wav"));
pub const VOICE_CT_WIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ct_win.wav"));
pub const VOICE_PLANTED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/planted.wav"));
pub const VOICE_DEFUSED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/defused.wav"));
pub const VOICE_GO: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/go.wav"));

/// Makes a clean voice recording sound like CS radio: telephone band, a little grit, and a click either side.
pub fn radio(wav_bytes: &[u8]) -> Vec<u8> {
    let data = wav_bytes.windows(4).position(|w| w == b"data").map_or(44, |i| i + 8);
    let pcm: Vec<f32> = wav_bytes[data.min(wav_bytes.len())..]
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / i16::MAX as f32)
        .collect();
    let (mut high, mut low, mut rng) = (0.0f32, 0.0f32, 0x5A5Au32);
    let mut out: Vec<f32> = Vec::with_capacity(pcm.len() + 2000);
    let click = |rng: &mut u32, out: &mut Vec<f32>| {
        for i in 0..600 {
            out.push(noise(rng) * (-(i as f32) / 90.0).exp() * 0.5);
        }
    };
    click(&mut rng, &mut out);
    for v in pcm {
        high += (v - high) * 0.04;
        low += ((v - high) - low) * 0.35;
        out.push((low * 3.0).tanh() * 0.9);
    }
    click(&mut rng, &mut out);
    wav(&out)
}

/// A footstep on sand and stone; four variants so a run doesn't sound like a metronome.
pub fn footstep(variant: u32) -> Vec<u8> {
    let (mut rng, mut low, mut phase) = (0xF00Du32 + variant * 7919, 0.0f32, 0.0f32);
    let thud = 70.0 + variant as f32 * 9.0;
    render(0.14, move |t| {
        low += (noise(&mut rng) - low) * 0.25;
        phase = (phase + thud / RATE as f32).fract();
        let scuff = low * (-t * 45.0).exp() * 1.4;
        scuff + (phase * TAU).sin() * (-t * 60.0).exp() * 0.6
    })
}

/// A bullet striking stone: a short sharp tick.
pub fn impact() -> Vec<u8> {
    let (mut rng, mut low) = (0x1A2Bu32, 0.0f32);
    render(0.08, move |t| {
        let n = noise(&mut rng);
        low += (n - low) * 0.3;
        (n - low) * (-t * 90.0).exp() * 1.2
    })
}

/// Swish: band of noise sweeping down.
pub fn knife() -> Vec<u8> {
    let (mut rng, mut low) = (0xBEEFu32, 0.0f32);
    render(0.22, move |t| {
        low += (noise(&mut rng) - low) * (0.5 - t * 1.8).max(0.05);
        low * (t / 0.22 * std::f32::consts::PI).sin() * 1.6
    })
}

/// Magazine out, magazine in.
pub fn reload() -> Vec<u8> {
    let mut rng = 0x5EEDu32;
    render(0.5, move |t| {
        let click = |at: f32| if t > at { (-(t - at) * 120.0).exp() } else { 0.0 };
        noise(&mut rng) * (click(0.0) + click(0.32) * 1.3) * 0.8
    })
}

/// Ka-ching.
pub fn buy() -> Vec<u8> {
    synth(0.35, |t| if t < 0.08 { 1500.0 } else { 2250.0 }, |t| (-(t % 0.08) * 6.0).exp() * (1.0 - t / 0.35), |p| (p * TAU).sin())
}

/// Metallic helmet ding.
pub fn headshot() -> Vec<u8> {
    render(0.4, |t| ((t * 3100.0 * TAU).sin() + (t * 4730.0 * TAU).sin() * 0.6) * (-t * 11.0).exp() * 0.5)
}

/// Short tick when your bullet lands.
pub fn hit_body() -> Vec<u8> {
    render(0.05, |t| (t * 900.0 * TAU).sin() * (1.0 - t / 0.05) * 0.6)
}

/// Dull thud when you take damage.
pub fn hurt() -> Vec<u8> {
    let (mut rng, mut low) = (0xD00Du32, 0.0f32);
    render(0.2, move |t| {
        low += (noise(&mut rng) - low) * 0.06;
        ((t * 110.0 * TAU).sin() * 0.7 + low * 3.0) * (-t * 18.0).exp()
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
    use crate::weapons::Weapon;

    #[test]
    fn valid_wav() {
        let w = super::death();
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize, w.len() - 8);
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize, w.len() - 44);
        if let Some(dir) = std::env::var_os("FPS16_DUMP_SFX") {
            let guns = [Weapon::Glock, Weapon::Usp, Weapon::Deagle, Weapon::Mp5, Weapon::Ak47, Weapon::M4a1, Weapon::Awp];
            let named = guns.map(|g| (g.spec().name, super::gunshot(g)));
            let extra = [("knife", super::knife()), ("reload", super::reload()), ("buy", super::buy()), ("headshot", super::headshot())];
            let extra = extra.into_iter().chain([("radio_go", super::radio(super::VOICE_GO)), ("radio_t_win", super::radio(super::VOICE_T_WIN))]);
            for (name, wav) in named.into_iter().chain(extra).chain([("hit", super::hit()), ("death", w)]) {
                std::fs::write(std::path::Path::new(&dir).join(format!("{name}.wav")), wav).unwrap();
            }
        }
    }
}
