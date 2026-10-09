//! Sound effects synthesized at startup, so the game stays a single binary with no asset files.

use std::f32::consts::TAU;

const RATE: u32 = 44100;

pub fn shoot() -> Vec<u8> {
    synth(0.3, |t| 2000.0 * (-t * 12.0).exp() + 300.0, |t| (-t * 10.0).exp(), |p| {
        let s = (p * TAU).sin();
        s.signum() * 0.4 + s * 0.6
    })
}

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

/// Renders a mono 16-bit WAV. `freq` and `amp` are functions of time; frequency is integrated so sweeps stay smooth.
fn synth(secs: f32, freq: impl Fn(f32) -> f32, amp: impl Fn(f32) -> f32, mut wave: impl FnMut(f32) -> f32) -> Vec<u8> {
    let n = (secs * RATE as f32) as u32;
    let mut phase = 0.0f32;
    let mut out = Vec::with_capacity(44 + n as usize * 2);
    for chunk in [&b"RIFF"[..], &(36 + n * 2).to_le_bytes(), b"WAVEfmt ", &16u32.to_le_bytes(), &1u16.to_le_bytes(),
        &1u16.to_le_bytes(), &RATE.to_le_bytes(), &(RATE * 2).to_le_bytes(), &2u16.to_le_bytes(), &16u16.to_le_bytes(),
        b"data", &(n * 2).to_le_bytes()]
    {
        out.extend_from_slice(chunk);
    }
    for i in 0..n {
        let t = i as f32 / RATE as f32;
        phase = (phase + freq(t) / RATE as f32).fract();
        let s = (wave(phase) * amp(t) * 0.5).clamp(-1.0, 1.0);
        out.extend_from_slice(&((s * i16::MAX as f32) as i16).to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn valid_wav() {
        let w = super::death();
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize, w.len() - 8);
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize, w.len() - 44);
        if let Some(dir) = std::env::var_os("FPS16_DUMP_SFX") {
            for (name, wav) in [("shoot", super::shoot()), ("hit", super::hit()), ("death", w)] {
                std::fs::write(std::path::Path::new(&dir).join(format!("{name}.wav")), wav).unwrap();
            }
        }
    }
}
