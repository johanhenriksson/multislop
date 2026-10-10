//! Abilities: anything a player uses, from guns to grenades to spells, as data run by one shared state machine.
//!
//! A use goes trigger -> cost -> timing -> delivery -> payload. Trigger, cost, timing and accuracy run in the predicted
//! step on both client and server (`Hands::tick`, `Hands::cone`); delivery and payload are the game's to resolve on
//! the server, since only it knows who is where.

use crate::TICK_HZ;
use glam::{EulerRot, Quat, Vec3};
use serde::{Deserialize, Serialize};

const DT: f32 = 1.0 / TICK_HZ as f32;
/// Bloom recovered per second.
const BLOOM_RECOVERY: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Trigger {
    /// Goes off while held.
    Auto,
    /// Once per press.
    Semi,
    /// Builds while held and goes off on release, at a strength from 0.5 to 1 reached after `full` seconds.
    Charge { full: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cost {
    Free,
    /// A magazine refilled from a reserve over `reload` seconds. Thrown items use the magazine as their count.
    Ammo { mag: u8, reserve: u16, reload: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Accuracy {
    /// Cone half-angle standing still, radians.
    pub spread: f32,
    /// Extra at full running speed.
    pub moving: f32,
    /// Extra while airborne.
    pub airborne: f32,
    /// View kick per use, radians. Kick builds up as bloom and widens the cone too.
    pub kick: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Delivery {
    /// Instant ray. Damage is multiplied by `falloff` per 12.7 m travelled.
    Hitscan { range: f32, falloff: f32 },
    Melee { reach: f32 },
    /// Thrown or launched: `speed` at full strength, bouncing with `bounce` of its speed kept, going off after `fuse`
    /// or, with `contact`, as soon as it touches anything (a rocket).
    Projectile { speed: f32, gravity: f32, bounce: f32, fuse: f32, contact: bool },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Payload {
    /// `armor_pen` is the fraction that gets through armor.
    Damage { amount: f32, armor_pen: f32 },
    /// Full damage at the center, falling off to nothing at `radius`.
    Explode { damage: f32, radius: f32, armor_pen: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ability {
    pub trigger: Trigger,
    pub cost: Cost,
    /// Seconds between uses.
    pub cycle: f32,
    pub accuracy: Accuracy,
    pub delivery: Delivery,
    pub payload: Payload,
}

impl Ability {
    /// A full magazine and reserve, or None if it costs nothing.
    pub fn full_ammo(&self) -> Option<(u8, u16)> {
        match self.cost {
            Cost::Free => None,
            Cost::Ammo { mag, reserve, .. } => Some((mag, reserve)),
        }
    }

    pub fn reload_secs(&self) -> f32 {
        match self.cost {
            Cost::Free => 0.0,
            Cost::Ammo { reload, .. } => reload,
        }
    }

    /// How far it reaches in a straight line (hitscan and melee).
    pub fn range(&self) -> f32 {
        match self.delivery {
            Delivery::Hitscan { range, .. } => range,
            Delivery::Melee { reach } => reach,
            Delivery::Projectile { .. } => 0.0,
        }
    }
}

/// What a player's hands are doing with the ability they hold. Part of the predicted state.
#[derive(Serialize, Deserialize, Clone, Copy, Default, Debug, PartialEq)]
pub struct Hands {
    /// Ticks until the next use is allowed (also covers drawing).
    pub cooldown: u16,
    /// Ticks left reloading, 0 when not.
    pub reloading: u16,
    /// Grows per use and recovers over time; drives view kick and spread.
    pub bloom: f32,
    /// Fire was held last tick.
    pub trigger: bool,
    /// Ticks the current charge has been held.
    pub charge: u16,
}

pub fn ticks(secs: f32) -> u16 {
    (secs * TICK_HZ as f32).round() as u16
}

impl Hands {
    /// Takes out an item: no use for `secs`, and any reload or charge is dropped.
    pub fn draw(&mut self, secs: f32) {
        (self.cooldown, self.reloading, self.charge) = (ticks(secs), 0, 0);
    }

    /// One tick holding `ability`, with `ammo` its (magazine, reserve) unless it costs nothing. `blocked` holds
    /// everything but reloads that are already running (freeze time). Returns the strength when it goes off.
    pub fn tick(&mut self, ability: &Ability, ammo: Option<&mut (u8, u16)>, fire: bool, reload: bool, blocked: bool) -> Option<f32> {
        self.cooldown = self.cooldown.saturating_sub(1);
        self.bloom = (self.bloom - DT * BLOOM_RECOVERY).max(0.0);
        let was_reloading = self.reloading > 0;
        if was_reloading {
            self.reloading -= 1;
        }
        let mag_size = ability.full_ammo().map_or(0, |a| a.0);
        let mut ammo = ammo;
        if let Some(ammo) = ammo.as_deref_mut() {
            let (mag, reserve) = *ammo;
            if was_reloading && self.reloading == 0 {
                let moved = (mag_size - mag).min(reserve.min(255) as u8);
                *ammo = (mag + moved, reserve - moved as u16);
            } else if !was_reloading && !blocked && reserve > 0 && mag < mag_size && (reload || (mag == 0 && fire)) {
                self.reloading = ticks(ability.reload_secs());
            }
        }

        let loaded = ammo.as_deref().is_none_or(|&(mag, _)| mag > 0);
        let ready = !blocked && self.cooldown == 0 && self.reloading == 0 && loaded;
        let strength = match ability.trigger {
            Trigger::Auto => (fire && ready).then_some(1.0),
            Trigger::Semi => (fire && ready && !self.trigger).then_some(1.0),
            Trigger::Charge { full } => {
                let held = self.charge;
                self.charge = if fire && ready { held + 1 } else { 0 };
                (!fire && held > 0 && ready).then(|| 0.5 + 0.5 * (held as f32 / ticks(full).max(1) as f32).min(1.0))
            }
        };
        self.trigger = fire;
        let strength = strength?;
        if let Some(ammo) = ammo {
            ammo.0 -= 1;
        }
        self.cooldown = ticks(ability.cycle).max(1);
        Some(strength)
    }

    /// View kick from bloom as (yaw, pitch) offsets. Uses follow it, so you pull down to hold a spray on target.
    pub fn punch(&self, ability: &Ability) -> (f32, f32) {
        let kick = ability.accuracy.kick;
        let up = self.bloom.min(10.0) * kick;
        let side = (self.bloom * 0.9).sin() * kick * 0.6 * (self.bloom / 3.0).min(1.0);
        (side, up)
    }

    /// Direction of a use that just went off from a view of `yaw`/`pitch` (forward is -Z): kicked and spread
    /// within the cone, which grows with `speed` (0..1 of run speed) and in the air. `spread` replaces the
    /// ability's base spread (a scope). Adds the use's bloom afterwards.
    #[allow(clippy::too_many_arguments)]
    pub fn aim(&mut self, ability: &Ability, spread: Option<f32>, speed: f32, on_ground: bool, yaw: f32, pitch: f32, seq: u32) -> Vec3 {
        let a = ability.accuracy;
        let air = if on_ground { 0.0 } else { a.airborne };
        let cone = spread.unwrap_or(a.spread) + speed * a.moving + air + self.bloom * a.kick * 0.15;
        let (sx, sy) = spread_offset(seq);
        let (side, up) = self.punch(ability);
        self.bloom += 1.0;
        Quat::from_euler(EulerRot::YXZ, yaw + side, pitch + up, 0.0) * Vec3::new(sx * cone, sy * cone, -1.0).normalize()
    }
}

/// Deterministic random point in the unit disk per use, so the client's predicted shots match the server's.
pub fn spread_offset(seq: u32) -> (f32, f32) {
    let mut x = seq.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x as f32 / u32::MAX as f32
    };
    let (r, a) = (next().sqrt(), next() * std::f32::consts::TAU);
    (r * a.cos(), r * a.sin())
}

/// Moves a thrown object one tick: gravity, then a bounce off whatever `trace(origin, dir) -> (distance, normal)`
/// reports in its way. Comes to rest once a bounce leaves it slow. Returns whether it hit something.
pub fn step_projectile(pos: &mut Vec3, vel: &mut Vec3, gravity: f32, bounce: f32, trace: impl Fn(Vec3, Vec3) -> (f32, Vec3)) -> bool {
    vel.y -= gravity * DT;
    let step = *vel * DT;
    let len = step.length();
    if len == 0.0 {
        return false;
    }
    let dir = step / len;
    let (t, normal) = trace(*pos, dir);
    if t > len {
        *pos += step;
        return false;
    }
    // Stop just short of the surface and reflect.
    *pos += dir * (t - 0.02).max(0.0);
    *vel = (*vel - 2.0 * vel.dot(normal) * normal) * bounce;
    if vel.length() < 1.0 {
        *vel = Vec3::ZERO;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUN: Ability = Ability {
        trigger: Trigger::Semi,
        cost: Cost::Ammo { mag: 2, reserve: 2, reload: 1.0 },
        cycle: 0.1,
        accuracy: Accuracy { spread: 0.0, moving: 0.0, airborne: 0.0, kick: 0.01 },
        delivery: Delivery::Hitscan { range: 100.0, falloff: 1.0 },
        payload: Payload::Damage { amount: 10.0, armor_pen: 1.0 },
    };

    /// Runs `secs` of ticks with fire held (or not), returning the strengths of every use.
    fn run(h: &mut Hands, a: &Ability, ammo: &mut (u8, u16), fire: bool, secs: f32) -> Vec<f32> {
        (0..ticks(secs)).filter_map(|_| h.tick(a, Some(ammo), fire, false, false)).collect()
    }

    #[test]
    fn semi_needs_a_press_per_use_then_reloads() {
        let (mut h, mut ammo) = (Hands::default(), (2, 2));
        assert_eq!(run(&mut h, &GUN, &mut ammo, true, 1.0).len(), 1, "held: one use");
        run(&mut h, &GUN, &mut ammo, false, 0.1);
        assert_eq!(run(&mut h, &GUN, &mut ammo, true, 0.05).len(), 1);
        assert_eq!(ammo, (0, 2));
        run(&mut h, &GUN, &mut ammo, false, 0.1);
        run(&mut h, &GUN, &mut ammo, true, 0.05);
        assert!(h.reloading > 0, "firing empty reloads");
        run(&mut h, &GUN, &mut ammo, false, 1.0);
        assert_eq!(ammo, (2, 0));
    }

    #[test]
    fn auto_fires_every_cycle() {
        let auto = Ability { trigger: Trigger::Auto, cost: Cost::Free, ..GUN };
        let mut h = Hands::default();
        let uses = (0..ticks(1.0)).filter_map(|_| h.tick(&auto, None, true, false, false)).count();
        assert_eq!(uses, 10);
    }

    #[test]
    fn charge_goes_off_on_release_stronger_when_held_longer() {
        let nade = Ability { trigger: Trigger::Charge { full: 1.0 }, cost: Cost::Ammo { mag: 2, reserve: 0, reload: 0.0 }, ..GUN };
        let (mut h, mut ammo) = (Hands::default(), (2, 0));
        assert!(run(&mut h, &nade, &mut ammo, true, 2.0).is_empty(), "nothing while held");
        let full = run(&mut h, &nade, &mut ammo, false, 0.1);
        run(&mut h, &nade, &mut ammo, true, 0.25);
        let quick = run(&mut h, &nade, &mut ammo, false, 0.1);
        assert_eq!((full, quick.len(), ammo), (vec![1.0], 1, (0, 0)));
        assert!(quick[0] > 0.5 && quick[0] < 0.7, "{quick:?}");
        run(&mut h, &nade, &mut ammo, true, 1.0);
        assert!(run(&mut h, &nade, &mut ammo, false, 0.1).is_empty(), "none left");
    }

    #[test]
    fn projectile_bounces_and_settles() {
        let floor = |o: Vec3, d: Vec3| if d.y < 0.0 { (o.y / -d.y, Vec3::Y) } else { (f32::MAX, Vec3::Y) };
        let (mut pos, mut vel) = (Vec3::new(0.0, 1.0, 0.0), Vec3::new(5.0, 3.0, 0.0));
        for _ in 0..ticks(3.0) {
            step_projectile(&mut pos, &mut vel, 20.0, 0.45, floor);
            assert!(pos.y >= 0.0);
        }
        assert_eq!(vel, Vec3::ZERO);
        assert!(pos.x > 1.0 && pos.y < 0.05, "{pos}");
    }
}
