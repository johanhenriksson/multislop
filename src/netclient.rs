//! Client netcode every game gets: clock sync with the server, the input stream, prediction of our own entity with
//! reconciliation, and interpolation of everyone else. The game only supplies its `predict` step.

use crate::net::*;
use bevy::math::Vec3;
use sdk::{Entity, Interpolated, TICK_HZ};
use std::collections::VecDeque;
use std::time::Instant;

/// One input (with its seq) applied to our entity: the game's client-side step. Runs once fresh from `input`, then
/// again for every unacked input on each snapshot.
pub type Predict<'a> = &'a mut dyn FnMut(&mut Entity, &[u8], u32);

/// Unacked inputs resent in every packet, so a lost packet costs nothing.
const REDUNDANCY: usize = 8;

/// Worst-case network stats, shown like CS's net_graph and reset by whoever shows them.
#[derive(Default)]
pub struct NetGraph {
    pub last_snap: Option<Instant>,
    pub max_gap_ms: f32,
    pub max_fix_cm: f32,
    pub starved: u32,
}

#[derive(Default)]
pub struct NetClient {
    /// Our entity id, once the server has told us.
    pub me: Option<u32>,
    /// Our entity as predicted up to the latest input.
    pub pred: Option<Entity>,
    pub ping_ms: f32,
    /// How far behind the newest snapshot remote entities are drawn. Adapts to jitter, never below `min_interp_ticks`.
    pub interp_ticks: f64,
    pub graph: NetGraph,
    seq: u32,
    pending: VecDeque<InputMsg>,
    /// Predicted position before the latest input, to draw in between fixed ticks.
    prev_pos: Vec3,
    /// Visual offset left over from reconciliation, decayed to zero so corrections don't snap.
    smooth: Vec3,
    latest_tick: u32,
    /// Server tick minus local clock in ticks. Estimating server time as local clock + offset keeps it advancing
    /// smoothly between snapshots instead of lurching each time one arrives.
    tick_offset: Option<f64>,
    snapshots: VecDeque<Snapshot>,
    min_interp_ticks: f64,
    /// Recent worst gap between snapshots in ticks, decaying slowly.
    jitter_ticks: f64,
}

impl NetClient {
    pub fn new(min_interp_ms: u64) -> NetClient {
        let min_interp_ticks = min_interp_ms as f64 / 1000.0 * TICK_HZ;
        NetClient {
            me: None,
            pred: None,
            ping_ms: 0.0,
            interp_ticks: min_interp_ticks,
            graph: NetGraph::default(),
            seq: 0,
            pending: VecDeque::new(),
            prev_pos: Vec3::ZERO,
            smooth: Vec3::ZERO,
            latest_tick: 0,
            tick_offset: None,
            snapshots: VecDeque::new(),
            min_interp_ticks,
            jitter_ticks: 0.0,
        }
    }

    /// Estimated current server tick (fractional).
    pub fn server_tick(&self) -> f64 {
        self.tick_offset.map_or(0.0, |o| clock() * TICK_HZ + o)
    }

    /// One tick of input: stamps it, predicts it and queues it for sending.
    pub fn input(&mut self, data: Vec<u8>, predict: Predict) {
        // Grow the buffer fast when snapshots arrive late, shrink it slowly once they don't.
        let target = (self.jitter_ticks + 2.0).clamp(self.min_interp_ticks, 0.15 * TICK_HZ);
        let rate = if target > self.interp_ticks { 0.2 } else { 0.005 };
        self.interp_ticks += (target - self.interp_ticks) * rate;

        self.seq += 1;
        let view_tick = (self.server_tick() - self.interp_ticks) as f32;
        if let Some(pred) = &mut self.pred {
            self.prev_pos = pred.pos;
            predict(pred, &data, self.seq);
        }
        self.pending.push_back(InputMsg { seq: self.seq, view_tick, data });
        while self.pending.len() > 128 {
            self.pending.pop_front();
        }
    }

    /// The packet to send this tick, if it's a sending tick.
    pub fn packet(&self, name: String) -> Option<InputPacket> {
        if self.seq % INPUT_EVERY != 0 {
            return None;
        }
        let inputs = self.pending.iter().skip(self.pending.len().saturating_sub(REDUNDANCY)).cloned().collect();
        let ping = self.ping_ms.round() as u16;
        Some(InputPacket { name, ping, time: clock(), inputs })
    }

    /// Takes in a snapshot and reconciles our prediction with it. Returns it unless it's older than one we have.
    pub fn snapshot(&mut self, snap: Snapshot, predict: Predict) -> Option<&Snapshot> {
        if snap.tick <= self.latest_tick {
            return None;
        }
        self.latest_tick = snap.tick;
        let sample = snap.tick as f64 - clock() * TICK_HZ;
        self.tick_offset = Some(match self.tick_offset {
            Some(o) if (sample - o).abs() < 8.0 => o + (sample - o) * 0.05,
            _ => sample,
        });
        self.me = Some(snap.you);
        if let Some(last) = self.graph.last_snap.replace(Instant::now()) {
            let gap = last.elapsed().as_secs_f64();
            self.graph.max_gap_ms = self.graph.max_gap_ms.max(gap as f32 * 1000.0);
            // Decays by elapsed time (half-life 2 s) so one hitch, e.g. while the window opens, doesn't linger.
            self.jitter_ticks = (gap * TICK_HZ).max(self.jitter_ticks * 0.5f64.powf(gap / 2.0));
        }
        if snap.echo > 0.0 {
            let rtt_ms = ((clock() - snap.echo) as f32 - snap.echo_held).max(0.0) * 1000.0;
            self.ping_ms = self.ping_ms * 0.9 + rtt_ms * 0.1;
        }

        if let Some(me) = snap.entities.iter().find(|e| e.id == snap.you) {
            while self.pending.front().is_some_and(|i| i.seq <= snap.ack) {
                self.pending.pop_front();
            }
            let mut s = me.clone();
            for inp in &self.pending {
                predict(&mut s, &inp.data, inp.seq);
            }
            if let Some(pred) = &self.pred {
                let err = pred.pos - s.pos;
                self.graph.max_fix_cm = self.graph.max_fix_cm.max(err.length() * 100.0);
                if err.length() < 2.0 && pred.teleport == s.teleport {
                    self.smooth += err;
                } else {
                    self.smooth = Vec3::ZERO;
                }
                self.prev_pos -= err;
            } else {
                self.prev_pos = s.pos;
            }
            self.pred = Some(s);
        }

        self.snapshots.push_back(snap);
        while self.snapshots.len() > 64 {
            self.snapshots.pop_front();
        }
        self.snapshots.back()
    }

    /// Where to draw our own entity this frame, `overstep` of the way to the next fixed tick. Decays the correction.
    pub fn render_pos(&mut self, overstep: f32, dt: f32) -> Vec3 {
        self.smooth *= (-dt * 15.0).exp();
        self.pred.as_ref().map_or(Vec3::ZERO, |p| self.prev_pos.lerp(p.pos, overstep) + self.smooth)
    }

    /// Everyone but us, as they should be drawn now: in the past by `interp_ticks`, between buffered snapshots.
    pub fn interpolate(&mut self) -> Vec<Interpolated> {
        let render_tick = self.server_tick() - self.interp_ticks;
        let Some(newest) = self.snapshots.back() else { return vec![] };
        if render_tick > newest.tick as f64 {
            self.graph.starved += 1;
        }
        let (a, b) = self
            .snapshots
            .iter()
            .zip(self.snapshots.iter().skip(1))
            .find(|(a, b)| a.tick as f64 <= render_tick && render_tick <= b.tick as f64)
            .unwrap_or((newest, newest));
        let f = if b.tick > a.tick { ((render_tick - a.tick as f64) / (b.tick - a.tick) as f64) as f32 } else { 0.0 };
        // Starved: keep entities moving on their last velocity for a moment rather than freezing them.
        let extrapolate = ((render_tick - newest.tick as f64) / TICK_HZ).clamp(0.0, 0.1) as f32;
        let find = |s: &Snapshot, id: u32| s.entities.iter().find(|e| e.id == id).cloned();
        b.entities
            .iter()
            .filter(|e| Some(e.id) != self.me)
            .map(|to| {
                let from = find(a, to.id).filter(|e| e.teleport == to.teleport).unwrap_or_else(|| to.clone());
                let latest = find(newest, to.id).map_or(to.teleport, |e| e.teleport);
                let entity = Entity {
                    pos: from.pos.lerp(to.pos, f) + to.vel * extrapolate,
                    yaw: from.yaw + wrap_angle(to.yaw - from.yaw) * f,
                    pitch: from.pitch + (to.pitch - from.pitch) * f,
                    ..to.clone()
                };
                let teleporting = latest != to.teleport || find(a, to.id).is_some_and(|e| e.teleport != to.teleport);
                Interpolated { entity, teleporting }
            })
            .collect()
    }
}

fn wrap_angle(a: f32) -> f32 {
    (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// Monotonic seconds since the client started.
pub fn clock() -> f64 {
    static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
    START.elapsed().as_secs_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(tick: u32, ack: u32, x: f32) -> Snapshot {
        let me = Entity { id: 7, pos: Vec3::new(x, 0.0, 0.0), ..Default::default() };
        Snapshot { tick, you: 7, ack, echo: 0.0, echo_held: 0.0, state: vec![], entities: vec![me], events: vec![] }
    }

    /// Each input moves the entity by its one byte along x.
    fn step(e: &mut Entity, input: &[u8], _seq: u32) {
        e.pos.x += input[0] as f32;
    }

    #[test]
    fn reconciles_by_replaying_unacked_inputs() {
        let mut nc = NetClient::new(50);
        nc.input(vec![1], &mut step);
        assert!(nc.pred.is_none(), "nothing to predict before the first snapshot");
        nc.snapshot(snap(10, 1, 0.0), &mut step).unwrap();
        for _ in 0..4 {
            nc.input(vec![1], &mut step);
        }
        assert_eq!(nc.pred.as_ref().unwrap().pos.x, 4.0);
        // The server ran inputs 1-3 but had us 1 m further: inputs 4 and 5 are replayed on top of its position.
        nc.snapshot(snap(14, 3, 3.0), &mut step).unwrap();
        assert_eq!(nc.pred.as_ref().unwrap().pos.x, 5.0);
        assert!(nc.snapshot(snap(12, 3, 0.0), &mut step).is_none(), "older snapshots are ignored");
        let sent = nc.packet("me".into()).map(|p| p.inputs.iter().map(|i| i.seq).collect::<Vec<_>>());
        assert_eq!(sent, None, "odd tick, nothing sent");
        nc.input(vec![1], &mut step);
        let sent = nc.packet("me".into()).unwrap().inputs.iter().map(|i| i.seq).collect::<Vec<_>>();
        assert_eq!(sent, [4, 5, 6], "every unacked input");
    }
}
