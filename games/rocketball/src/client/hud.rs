//! The HUD: a Rocket League style score bug with the clock, health and armor, the weapon in hand with its load and
//! reserve, the crosshair (ringed when a rocket would lock onto the ball) or the AWP's scope, a marker over the ball, the kill feed, big centered messages (kickoff count, goals, the winner), the
//! respawn count and the scoreboard (Tab, and at the end of a match).

use super::{BallClient, ORANGE, BLUE, team_text};
use crate::shared::{BALL_RADIUS, Gun, NetPlayer, Phase, Team};
use glam::Vec3;
use sdk::client::*;

const TEXT: Color = Color::srgba(1.0, 1.0, 1.0, 0.94);
const DIM: Color = Color::srgba(1.0, 1.0, 1.0, 0.25);
const PANEL: Color = Color::srgba(0.01, 0.02, 0.06, 0.6);
const HEALTH: Color = Color::srgb(0.4, 1.0, 0.5);
const ARMOR: Color = Color::srgb(1.0, 0.8, 0.25);
const LOW: Color = Color::srgb(1.0, 0.3, 0.25);
const ROCKET_LIT: Color = Color::srgb(1.0, 0.75, 0.35);
const LOCK: Color = Color::srgb(1.0, 0.25, 0.2);

/// HUD node ids.
#[derive(Default)]
pub struct Hud {
    score: [u32; 2],
    clock: u32,
    overtime: u32,
    health: u32,
    armor: u32,
    /// One per round in the biggest magazine (the AWP's 5).
    pips: [u32; 5],
    crosshair: u32,
    lock: u32,
    scope: u32,
    reserve: u32,
    reload: u32,
    marker: u32,
    marker_ring: u32,
    marker_dot: u32,
    feed: [u32; 5],
    big: u32,
    sub: u32,
    note: u32,
    respawn: u32,
    hurt: u32,
    board: u32,
    board_title: u32,
    sections: [u32; 2],
}

fn node() -> UiNode {
    UiNode::default()
}

fn text(size: f32, color: Color) -> UiNode {
    UiNode { text: Some(Text { shadow: true, ..Text::new("", size, color) }), ..node() }
}

fn centered(size: f32, color: Color) -> UiNode {
    UiNode { text: Some(Text { shadow: true, center: true, ..Text::new("", size, color) }), ..node() }
}

/// A full-screen layer, children centered.
fn screen() -> UiNode {
    UiNode { absolute: true, width: percent(100.0), height: percent(100.0), justify: Align::Center, align: Align::Center, ..node() }
}

/// A full-width row this far down the screen, its children centered.
fn row_at(top: f32) -> UiNode {
    UiNode { absolute: true, top: percent(top), width: percent(100.0), justify: Align::Center, ..node() }
}

fn srgba(c: [f32; 3], a: f32) -> Color {
    Color::srgba(c[0], c[1], c[2], a)
}

impl BallClient {
    pub(super) fn build_hud(&mut self) {
        let s = &mut self.scene;
        let h = &mut self.hud;
        h.hurt = s.ui(None, UiNode { absolute: true, width: percent(100.0), height: percent(100.0), ..node() });

        // The score bug: Orange's score, the clock, Blue's score; OVERTIME hangs under the clock.
        let top = s.ui(None, UiNode { absolute: true, top: px(12.0), width: percent(100.0), justify: Align::Center, ..node() });
        let bug = s.ui(Some(top), UiNode { column: true, align: Align::Center, ..node() });
        let bar = s.ui(Some(bug), UiNode { align: Align::Center, ..node() });
        let boxed = |w: f32, color: Color, radius: Corners| UiNode {
            width: px(w),
            height: px(52.0),
            justify: Align::Center,
            align: Align::Center,
            background: color,
            radius,
            ..node()
        };
        let left = Corners { top_left: 10.0, bottom_left: 10.0, ..Corners::default() };
        let right = Corners { top_right: 10.0, bottom_right: 10.0, ..Corners::default() };
        let orange = s.ui(Some(bar), boxed(74.0, srgba(ORANGE, 0.92), left));
        h.score[Team::Orange as usize] = s.ui(Some(orange), centered(38.0, Color::WHITE));
        let clock = s.ui(Some(bar), UiNode { border: Edges::axes(0.0, 2.0), border_color: Color::srgba(1.0, 1.0, 1.0, 0.12), ..boxed(124.0, Color::srgba(0.02, 0.03, 0.08, 0.88), Corners::default()) });
        h.clock = s.ui(Some(clock), centered(30.0, TEXT));
        let blue = s.ui(Some(bar), boxed(74.0, srgba(BLUE, 0.92), right));
        h.score[Team::Blue as usize] = s.ui(Some(blue), centered(38.0, Color::WHITE));
        let overtime = UiNode { padding: Edges::axes(14.0, 3.0), background: Color::srgba(0.85, 0.1, 0.12, 0.92), radius: Corners::bottom(6.0), hidden: true, ..node() };
        h.overtime = s.ui(Some(bug), overtime);
        s.ui(Some(h.overtime), UiNode { text: Some(Text::new("OVERTIME", 15.0, Color::WHITE)), ..node() });

        // Crosshair: a dot and four ticks around the exact center.
        h.crosshair = s.ui(None, screen());
        let anchor = s.ui(Some(h.crosshair), UiNode { width: px(0.0), height: px(0.0), ..node() });
        let tick = |left: f32, top: f32, w: f32, h: f32| UiNode {
            absolute: true,
            left: px(left),
            top: px(top),
            width: px(w),
            height: px(h),
            background: TEXT,
            outline: Some((1.0, Color::srgba(0.0, 0.0, 0.0, 0.5))),
            ..node()
        };
        for t in [tick(-1.0, -1.0, 2.0, 2.0), tick(-13.0, -1.0, 7.0, 2.0), tick(6.0, -1.0, 7.0, 2.0), tick(-1.0, -13.0, 2.0, 7.0), tick(-1.0, 6.0, 2.0, 7.0)] {
            s.ui(Some(anchor), t);
        }
        // Lock-on: a red ring around it and LOCK under it.
        h.lock = s.ui(Some(anchor), UiNode { hidden: true, ..node() });
        let ring = UiNode {
            absolute: true,
            left: px(-22.0),
            top: px(-22.0),
            width: px(44.0),
            height: px(44.0),
            border: Edges::all(3.0),
            border_color: LOCK,
            radius: Corners::all(1e4),
            outline: Some((1.0, Color::srgba(0.0, 0.0, 0.0, 0.5))),
            ..node()
        };
        s.ui(Some(h.lock), ring);
        let label = UiNode { absolute: true, left: px(-40.0), top: px(26.0), width: px(80.0), ..centered(15.0, LOCK) };
        let label = s.ui(Some(h.lock), label);
        s.ui_text(label, "LOCK");

        // Dust's scope: a round lens whose huge black outline blacks out the rest of the screen, with thin crosshair
        // lines through it.
        h.scope = s.ui(None, UiNode { hidden: true, ..screen() });
        let lens = UiNode { width: vh(96.0), height: vh(96.0), radius: Corners::all(1e5), outline: Some((4000.0, Color::BLACK)), ..node() };
        let lens = s.ui(Some(h.scope), lens);
        for horizontal in [true, false] {
            let hair = if horizontal {
                UiNode { left: percent(0.0), top: percent(50.0), width: percent(100.0), height: px(2.0), margin: Edges::top(-1.0), ..node() }
            } else {
                UiNode { left: percent(50.0), top: percent(0.0), width: px(2.0), height: percent(100.0), margin: Edges::left(-1.0), ..node() }
            };
            s.ui(Some(lens), UiNode { absolute: true, background: Color::BLACK, ..hair });
        }

        // Health and armor, bottom left.
        let panel = |left: Option<f32>| UiNode {
            absolute: true,
            left: left.map_or(Val::Auto, px),
            right: if left.is_none() { px(24.0) } else { Val::Auto },
            bottom: px(20.0),
            padding: Edges::axes(18.0, 8.0),
            gap: 12.0,
            align: Align::Center,
            background: PANEL,
            radius: Corners::all(10.0),
            ..node()
        };
        let vitals = s.ui(None, panel(Some(24.0)));
        let cross = s.ui(Some(vitals), UiNode { width: px(26.0), height: px(26.0), justify: Align::Center, align: Align::Center, ..node() });
        s.ui(Some(cross), UiNode { width: px(26.0), height: px(8.0), background: HEALTH, ..node() });
        s.ui(Some(cross), UiNode { absolute: true, width: px(8.0), height: px(26.0), background: HEALTH, ..node() });
        h.health = s.ui(Some(vitals), UiNode { width: px(76.0), ..text(42.0, TEXT) });
        let shield = UiNode { width: px(20.0), height: px(24.0), border: Edges::all(4.0), radius: Corners::bottom(10.0), border_color: ARMOR, ..node() };
        s.ui(Some(vitals), shield);
        h.armor = s.ui(Some(vitals), UiNode { width: px(76.0), ..text(42.0, TEXT) });

        // The gun in hand, bottom right: its name, the load as pips, the reserve as a number.
        let reload = UiNode { absolute: true, right: px(28.0), bottom: px(86.0), ..text(17.0, ROCKET_LIT) };
        h.reload = s.ui(None, reload);
        let ammo = s.ui(None, panel(None));
        let pips = s.ui(Some(ammo), UiNode { gap: 5.0, align: Align::Center, ..node() });
        h.pips = [0; 5].map(|_| s.ui(Some(pips), UiNode { width: px(11.0), height: px(32.0), radius: Corners { top_left: 6.0, top_right: 6.0, bottom_left: 2.0, bottom_right: 2.0 }, background: ROCKET_LIT, ..node() }));
        s.ui(Some(ammo), UiNode { width: px(2.0), height: px(32.0), background: DIM, ..node() });
        h.reserve = s.ui(Some(ammo), UiNode { width: px(54.0), ..text(42.0, TEXT) });

        // A ring over the ball, its dot in the last toucher's color, kept on screen through cover.
        let marker = UiNode {
            absolute: true,
            width: px(18.0),
            height: px(18.0),
            justify: Align::Center,
            align: Align::Center,
            border: Edges::all(2.0),
            border_color: Color::srgba(1.0, 1.0, 1.0, 0.85),
            radius: Corners::all(1e4),
            world: Some(Vec3::ZERO),
            ..node()
        };
        // Hidden through a parent: the host keeps a world-anchored node's own visibility for itself.
        h.marker = s.ui(None, UiNode { absolute: true, width: percent(100.0), height: percent(100.0), hidden: true, ..node() });
        let marker = s.ui(Some(h.marker), marker);
        h.marker_ring = marker;
        h.marker_dot = s.ui(Some(marker), UiNode { width: px(8.0), height: px(8.0), radius: Corners::all(1e4), background: TEXT, ..node() });

        let feed = s.ui(None, UiNode { absolute: true, right: px(20.0), top: px(16.0), column: true, align: Align::End, gap: 4.0, ..node() });
        h.feed = [0; 5].map(|_| s.ui(Some(feed), text(16.0, TEXT)));

        let middle = s.ui(None, UiNode { column: true, align: Align::Center, ..row_at(20.0) });
        h.big = s.ui(Some(middle), centered(110.0, TEXT));
        h.sub = s.ui(Some(middle), centered(30.0, TEXT));
        let lower = s.ui(None, UiNode { column: true, align: Align::Center, gap: 8.0, ..row_at(60.0) });
        h.note = s.ui(Some(lower), centered(22.0, Color::srgb(1.0, 0.85, 0.35)));
        h.respawn = s.ui(Some(lower), centered(26.0, TEXT));

        h.board = s.ui(None, UiNode { hidden: true, ..screen() });
        let panel = UiNode {
            column: true,
            gap: 16.0,
            padding: Edges::all(22.0),
            radius: Corners::all(8.0),
            background: Color::srgba(0.0, 0.01, 0.05, 0.9),
            ..node()
        };
        let panel = s.ui(Some(h.board), panel);
        h.board_title = s.ui(Some(panel), centered(26.0, TEXT));
        h.sections = [Team::Orange, Team::Blue].map(|t| s.ui(Some(panel), text(18.0, team_text(t))));
    }

    pub(super) fn update_hud(&mut self) {
        let m = self.state.clone();
        let respawn_in = self.respawn_in();
        let (h, s) = (&self.hud, &mut self.scene);
        let score = m.as_ref().map_or([0; 2], |m| m.score);
        for t in [Team::Orange, Team::Blue] {
            s.ui_text(h.score[t as usize], score[t as usize].to_string());
        }
        let secs = m.as_ref().map_or(0, |m| m.time_left.ceil() as u32);
        s.ui_text(h.clock, format!("{}:{:02}", secs / 60, secs % 60));
        s.ui_hidden(h.overtime, !m.as_ref().is_some_and(|m| m.overtime));

        let alive = self.alive && self.me.is_some();
        let low = |v: i32, at: i32| if v <= at { LOW } else { TEXT };
        let (hp, armor) = (self.hp, self.armor);
        s.ui_set(h.health, |n| {
            let t = n.text.as_mut().unwrap();
            (t.value, t.color) = (if alive { hp.to_string() } else { "0".into() }, low(hp, 25));
        });
        s.ui_text(h.armor, armor.to_string());
        let (gun, (loaded, reserve)) = (self.pred.gun, self.pred.ammo);
        let mag = gun.ability().full_ammo().map_or(0, |a| a.0);
        for (i, &pip) in h.pips.iter().enumerate() {
            s.ui_set(pip, |n| (n.background, n.hidden) = (if (i as u8) < loaded && alive { ROCKET_LIT } else { DIM }, i as u8 >= mag));
        }
        let scoped = self.zoom && alive;
        s.ui_hidden(h.scope, !scoped);
        s.ui_hidden(h.crosshair, scoped);
        s.ui_hidden(h.lock, !self.locked);
        let knife = gun == Gun::Knife;
        s.ui_set(h.reserve, |n| {
            let t = n.text.as_mut().unwrap();
            (t.value, t.color) = (if knife { "-".into() } else { reserve.to_string() }, if reserve == 0 && !knife { LOW } else { TEXT });
        });
        let reloading = self.pred.hands.reloading > 0;
        let reload = match gun {
            _ if !alive => "",
            Gun::Knife if self.pred.primary.is_none() => "3  KNIFE   (no primary: walk over one)",
            Gun::Knife => "3  KNIFE",
            _ if reloading => "RELOADING",
            _ if loaded == 0 && reserve == 0 => "OUT OF AMMO",
            _ if loaded == 0 => "R TO RELOAD",
            Gun::Awp => "1  AWP",
            Gun::Launcher => "1  LAUNCHER",
        };
        s.ui_text(h.reload, reload);

        let c = m.as_ref().and_then(|m| m.last_touch).map_or(super::NEUTRAL, super::team_rgb);
        let ball = self.ball.pos + Vec3::Y * (BALL_RADIUS + 0.9);
        let shown = self.ball.shown && self.ball.touch.is_some();
        s.ui_hidden(h.marker, !shown);
        s.ui_set(h.marker_ring, |n| n.world = Some(ball));
        s.ui_set(h.marker_dot, |n| n.background = srgba(c, 0.95));

        while self.feed.front().is_some_and(|(_, _, at)| self.time - at > 6.0) {
            self.feed.pop_front();
        }
        for (i, &line) in h.feed.iter().enumerate() {
            let (value, team) = self.feed.get(i).map_or((String::new(), None), |(l, t, _)| (l.clone(), *t));
            s.ui_set(line, |n| {
                let t = n.text.as_mut().unwrap();
                (t.value, t.color) = (value, team.map_or(TEXT, team_text));
            });
        }

        let name = |id: Option<u32>| id.and_then(|id| self.players.iter().find(|p| p.id == id)).map(|p| p.name.clone());
        let recent = self.message.as_ref().filter(|(_, at)| self.time - at < 1.5).map(|(m, _)| m.as_str());
        let (big, sub, color) = match m.as_ref().map(|m| m.phase) {
            Some(Phase::Kickoff { countdown }) => (format!("{}", countdown.ceil().max(1.0)), "KICKOFF".to_string(), TEXT),
            Some(Phase::Goal { team, scorer, own_goal }) => {
                let sub = match (name(scorer), own_goal) {
                    (Some(n), true) => format!("Own goal by {n}"),
                    (Some(n), false) => format!("{n} scored!"),
                    (None, _) => format!("{} scores!", if team == Team::Orange { "Orange" } else { "Blue" }),
                };
                ("GOAL!".to_string(), sub, team_text(team))
            }
            Some(Phase::End { winner }) => {
                let big = match winner {
                    Some(Team::Orange) => "ORANGE WINS",
                    Some(Team::Blue) => "BLUE WINS",
                    None => "DRAW",
                };
                (big.to_string(), String::new(), winner.map_or(TEXT, team_text))
            }
            _ if recent == Some("GO!") => ("GO!".to_string(), String::new(), TEXT),
            _ => (String::new(), String::new(), TEXT),
        };
        s.ui_set(h.big, |n| {
            let t = n.text.as_mut().unwrap();
            (t.value, t.color) = (big, color);
        });
        s.ui_text(h.sub, sub);
        let note = self.message.as_ref().filter(|(m, at)| m != "GO!" && self.time - at < 2.0).map_or(String::new(), |(m, _)| m.clone());
        s.ui_text(h.note, note);
        let ended = matches!(m.as_ref().map(|m| m.phase), Some(Phase::End { .. }));
        let respawn = respawn_in.filter(|_| !ended).map_or(String::new(), |n| format!("Respawning in {n}"));
        s.ui_text(h.respawn, respawn);

        // A red flash when hit, in steps so it isn't resent every frame.
        let hurt = (1.0 - (self.time - self.hurt_at) as f32 / 0.4).max(0.0);
        let hurt = (hurt * 6.0).round() / 6.0 * 0.3;
        s.ui_set(h.hurt, |n| n.background = Color::srgba(0.8, 0.0, 0.0, hurt));

        let show = self.tab || ended;
        s.ui_hidden(h.board, !show);
        if show {
            s.ui_text(h.board_title, format!("ORANGE  {}  -  {}  BLUE", score[0], score[1]));
            for t in [Team::Orange, Team::Blue] {
                let mut team: Vec<&NetPlayer> = self.players.iter().filter(|p| p.team == t).collect();
                team.sort_by_key(|p| (std::cmp::Reverse(p.goals), std::cmp::Reverse(p.kills), p.deaths));
                let mut out = format!("  {:<20}{:>7}{:>7}{:>8}{:>7}", if t == Team::Orange { "ORANGE" } else { "BLUE" }, "Goals", "Kills", "Deaths", "Ping");
                for p in team {
                    let you = if Some(p.id) == self.me { ">" } else { " " };
                    let ping = if p.bot { "BOT".into() } else { p.ping.to_string() };
                    out += &format!("\n{you} {:<20}{:>7}{:>7}{:>8}{:>7}", p.name, p.goals, p.kills, p.deaths, ping);
                }
                s.ui_text(h.sections[t as usize], out);
            }
        }
    }
}
