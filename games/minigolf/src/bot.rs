//! Server-side bots. With the ball at rest, a bot tries shots in every direction at a few strengths (plus putts at
//! the cup) by running them through `step_ball`, a few per tick while it waits, takes the one that ends nearest the
//! cup along the hole even when hit a little off, then shoots it a little off. They produce an `Input` per tick like a client would.

use crate::map::course;
use crate::shared::*;
use std::f32::consts::TAU;

/// Directions and strengths tried, and putts straight at the cup at these strengths.
const YAWS: usize = 36;
const POWERS: [f32; 6] = [0.2, 0.35, 0.5, 0.65, 0.8, 1.0];
const PUTTS: [f32; 8] = [0.04, 0.07, 0.1, 0.14, 0.19, 0.25, 0.32, 0.4];
const CANDIDATES: usize = YAWS * POWERS.len() + PUTTS.len();
/// Shots tried per tick (each three times), so no tick takes long; all of them take under `WAIT.0`.
const PER_TICK: usize = 2;
/// How long a tried shot is followed, at most.
const SIM_TICKS: u32 = TICK_HZ as u32 * 12;
/// How far off a bot hits what it meant: radians of aim, and share of power. The difficulty knobs.
const AIM_ERROR: f32 = 0.05;
const POWER_ERROR: f32 = 0.08;
/// Seconds a bot takes over a shot.
const WAIT: (f32, f32) = (1.0, 3.0);

struct Plan {
    /// The ball's clock when it'll shoot, so tried shots meet the windmills where the real one will.
    at: u32,
    tried: usize,
    /// Score, yaw and power of the best shot so far.
    best: (f32, f32, f32),
}

pub struct Bot {
    rng: u32,
    plan: Option<Plan>,
}

impl Bot {
    pub fn new(seed: u32) -> Bot {
        Bot { rng: seed.wrapping_mul(0x9E37_79B9) | 1, plan: None }
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }

    pub fn think(&mut self, b: &Ball) -> Input {
        let idle = Input { yaw: b.yaw, ..Default::default() };
        if !b.can_shoot() {
            self.plan = None;
            return idle;
        }
        if self.plan.is_none() {
            let wait = WAIT.0 + self.random() * (WAIT.1 - WAIT.0);
            self.plan = Some(Plan { at: b.clock + (wait * TICK_HZ as f32) as u32, tried: 0, best: (f32::MAX, 0.0, 0.5) });
        }
        let plan = self.plan.as_mut().unwrap();
        for _ in 0..PER_TICK.min(CANDIDATES - plan.tried) {
            let (yaw, power) = candidate(b, plan.tried);
            // Tried as meant and as missed either way, so a shot only good when hit perfectly isn't taken.
            let score = [0.0, 1.0, -1.0].iter().map(|&e| try_shot(b, plan.at, yaw + e * AIM_ERROR, power * (1.0 + e * POWER_ERROR))).sum::<f32>();
            if score < plan.best.0 {
                plan.best = (score, yaw, power);
            }
            plan.tried += 1;
        }
        if plan.tried < CANDIDATES || b.clock + 1 < plan.at {
            return idle;
        }
        let (_, yaw, power) = self.plan.take().unwrap().best;
        let yaw = yaw + (self.random() * 2.0 - 1.0) * AIM_ERROR;
        let power = power * (1.0 + (self.random() * 2.0 - 1.0) * POWER_ERROR);
        Input { yaw, shot: power.clamp(0.02, 1.0), ..Default::default() }
    }
}

/// The `i`th shot to try from `b`.
fn candidate(b: &Ball, i: usize) -> (f32, f32) {
    if i < YAWS * POWERS.len() {
        return (i as f32 * TAU / YAWS as f32, POWERS[i / YAWS]);
    }
    let to = course().hole_at(b.pos).cup - b.pos;
    (f32::atan2(-to.x, -to.z), PUTTS[i - YAWS * POWERS.len()])
}

/// How a shot ends up, lower being better: under 0 in the cup, otherwise cells to the cup along the hole plus a bit
/// for the straight distance; going out is worst.
fn try_shot(b: &Ball, at: u32, yaw: f32, power: f32) -> f32 {
    let mut ball = Ball { clock: at - 1, ..*b };
    let course = course();
    let hole = course.hole_at(b.pos);
    let mut inp = Input { yaw, shot: power, ..Default::default() };
    for _ in 0..SIM_TICKS {
        let step = step_ball(&mut ball, &inp);
        inp.shot = 0.0;
        if step.holed {
            return -1.0;
        }
        if step.out.is_some() {
            return 10_000.0;
        }
        if ball.at_rest() {
            break;
        }
    }
    hole.to_cup(ball.pos) as f32 + 0.3 * ball.pos.distance(hole.cup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{self, HOLES};

    /// A bot alone on each hole of a few courses: it should sink most of them in a sane number of strokes.
    #[test]
    fn bots_sink_holes() {
        let mut total = (0, 0, 0);
        for seed in [1, 2, 3] {
            map::load(&format!("random-{seed}"));
            for h in 0..HOLES {
                let hole = &course().holes[h];
                let mut bot = Bot::new(seed * 10 + h as u32);
                let mut ball = Ball::on_tee(hole.tee, false);
                for _ in 0..TICK_HZ as u32 * 120 {
                    let inp = bot.think(&ball);
                    step_ball(&mut ball, &inp);
                    if ball.holed || ball.strokes >= MAX_STROKES && ball.at_rest() {
                        break;
                    }
                }
                println!("seed {seed} hole {}: par {}, {} strokes, holed {}", h + 1, hole.par, ball.strokes, ball.holed);
                total.0 += ball.holed as u32;
                total.1 += ball.strokes as u32;
                total.2 += hole.par as u32;
            }
        }
        println!("{} of {} holed, {} strokes for par {}", total.0, 3 * HOLES, total.1, total.2);
        assert!(total.0 as usize >= 3 * HOLES * 8 / 10, "only {} of {} holed", total.0, 3 * HOLES);
        assert!(total.1 < total.2 * 2, "{} strokes for par {}", total.1, total.2);
    }
}
