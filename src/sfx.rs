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

/// A gunshot: low-passed noise for the crack plus a falling sine for the thump, shaped per weapon.
pub fn gunshot(weapon: Weapon) -> Vec<u8> {
    // (length, thump Hz, noise brightness 0..1, loudness)
    let (secs, thump, bright, loud) = match weapon {
        Weapon::Knife => return knife(),
        Weapon::Glock => (0.2, 190.0, 0.55, 0.7),
        Weapon::Usp => (0.22, 160.0, 0.45, 0.7),
        Weapon::Deagle => (0.5, 85.0, 0.4, 1.0),
        Weapon::Mp5 => (0.16, 210.0, 0.6, 0.6),
        Weapon::Ak47 => (0.38, 105.0, 0.35, 0.95),
        Weapon::M4a1 => (0.32, 130.0, 0.45, 0.85),
        Weapon::Awp => (1.0, 65.0, 0.3, 1.0),
    };
    let (mut rng, mut low, mut phase) = (0xC0FF_EE11u32 ^ thump as u32, 0.0f32, 0.0f32);
    render(secs, move |t| {
        let crack = (-t * 38.0 / secs).exp();
        let tail = (-t * 6.0 / secs).exp() * 0.35;
        low += (noise(&mut rng) - low) * bright;
        phase = (phase + thump * (1.0 + 2.0 * (-t * 30.0).exp()) / RATE as f32).fract();
        let boom = (phase * TAU).sin() * (-t * 9.0 / secs).exp();
        (low * 2.2 * (crack + tail) + boom * 0.9) * loud
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
            for (name, wav) in named.into_iter().chain(extra).chain([("hit", super::hit()), ("death", w)]) {
                std::fs::write(std::path::Path::new(&dir).join(format!("{name}.wav")), wav).unwrap();
            }
        }
    }
}
