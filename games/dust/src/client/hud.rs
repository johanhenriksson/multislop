//! The CS 1.6 HUD: amber health, armor, money, ammo and timer, dynamic crosshair, kill feed, buy and radio menus, Tab
//! scoreboard, scope, announcements, damage flash and death fade.

use super::{CROSSHAIR_GREEN, DustClient, HUD_AMBER, TEXT_SIZE, team_text_color};
use crate::shared::*;
use crate::weapons::buy_menu;
use glam::Vec2;
use sdk::client::*;

fn node() -> UiNode {
    UiNode::default()
}

/// A full-screen layer, children centered.
fn screen() -> UiNode {
    UiNode { absolute: true, width: percent(100.0), height: percent(100.0), justify: Align::Center, align: Align::Center, ..node() }
}

/// A full-width row at a height, its child centered.
fn centered_row(top: Val, bottom: Val) -> UiNode {
    UiNode { absolute: true, top, bottom, width: percent(100.0), justify: Align::Center, ..node() }
}

fn text(value: &str, size: f32, color: Color) -> UiNode {
    UiNode { text: Some(Text::new(value, size, color)), ..node() }
}

fn shadowed(size: f32) -> UiNode {
    UiNode { text: Some(Text { shadow: true, ..Text::new("", size, Color::WHITE) }), ..node() }
}

fn panel() -> UiNode {
    UiNode {
        absolute: true,
        left: px(24.0),
        top: percent(28.0),
        padding: Edges::all(14.0),
        border: Edges::left(3.0),
        background: Color::srgba(0.0, 0.0, 0.0, 0.75),
        border_color: HUD_AMBER,
        hidden: true,
        ..node()
    }
}

impl DustClient {
    pub(super) fn build_hud(&mut self) {
        let s = &mut self.scene;
        let h = &mut self.hud;
        let top = UiNode { text: Some(Text { center: true, ..Text::new("", TEXT_SIZE, Color::WHITE) }), ..centered_row(px(12.0), Val::Auto) };
        h.scoreboard = s.ui(None, top);
        h.death_fade = s.ui(None, UiNode { absolute: true, width: percent(100.0), height: percent(100.0), ..node() });
        // A zero-size anchor at the exact center; the crosshair lines are placed around it.
        let layer = s.ui(None, screen());
        let anchor = s.ui(Some(layer), UiNode { width: px(0.0), height: px(0.0), ..node() });
        h.crosshair = [Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::NEG_Y].map(|dir| (s.ui(Some(anchor), UiNode { absolute: true, background: CROSSHAIR_GREEN, ..node() }), dir));

        // CS 1.6 HUD: amber health and armor bottom-left, money and ammo bottom-right; status messages centered.
        let big = |value: &str| text(value, 34.0, HUD_AMBER);
        let row = s.ui(None, UiNode { absolute: true, left: px(20.0), bottom: px(14.0), align: Align::Center, gap: 10.0, ..node() });
        // Health cross.
        let cross = s.ui(Some(row), UiNode { width: px(24.0), height: px(24.0), justify: Align::Center, align: Align::Center, ..node() });
        s.ui(Some(cross), UiNode { width: px(24.0), height: px(7.0), background: HUD_AMBER, ..node() });
        s.ui(Some(cross), UiNode { absolute: true, width: px(7.0), height: px(24.0), background: HUD_AMBER, ..node() });
        h.health = s.ui(Some(row), UiNode { width: px(80.0), ..big("") });
        // Armor: a shield-ish outline.
        let shield = UiNode { width: px(18.0), height: px(22.0), border: Edges::all(4.0), radius: Corners::bottom(9.0), border_color: HUD_AMBER, ..node() };
        s.ui(Some(row), shield);
        h.armor = s.ui(Some(row), big(""));
        let money = s.ui(None, UiNode { absolute: true, right: px(20.0), bottom: px(56.0), ..node() });
        h.money = s.ui(Some(money), big(""));
        let timer = s.ui(None, centered_row(Val::Auto, px(14.0)));
        h.timer = s.ui(Some(timer), big(""));
        let status = UiNode { text: Some(Text { center: true, ..Text::new("", TEXT_SIZE, Color::WHITE) }), ..centered_row(Val::Auto, px(90.0)) };
        h.status = s.ui(None, status);

        h.score_panel = s.ui(None, UiNode { hidden: true, ..screen() });
        let board = UiNode { column: true, gap: 18.0, padding: Edges::all(20.0), radius: Corners::all(6.0), background: Color::srgba(0.0, 0.0, 0.0, 0.78), ..node() };
        let board = s.ui(Some(h.score_panel), board);
        h.sections = [Color::srgb(1.0, 0.45, 0.3), Color::srgb(0.55, 0.7, 1.0)].map(|c| s.ui(Some(board), text("", 17.0, c)));

        let announce = s.ui(None, centered_row(percent(26.0), Val::Auto));
        h.announce = s.ui(Some(announce), shadowed(40.0));
        let radio_lines: String = std::iter::once("RADIO".to_string())
            .chain(Radio::MENU.iter().enumerate().map(|(i, r)| format!("{}  {}", i + 1, r.text())))
            .collect::<Vec<_>>()
            .join("\n");
        h.radio_menu = s.ui(None, UiNode { text: Some(Text::new(radio_lines, 18.0, Color::srgb(1.0, 0.85, 0.35))), ..panel() });
        let target = s.ui(None, centered_row(percent(58.0), Val::Auto));
        h.target = s.ui(Some(target), shadowed(18.0));
        let chat = Text { shadow: true, ..Text::new("", 16.0, Color::srgb(1.0, 0.85, 0.35)) };
        h.chat = s.ui(None, UiNode { absolute: true, left: px(20.0), bottom: px(70.0), text: Some(chat), ..node() });

        // Scope: a round lens whose huge black outline blacks out the rest of the screen, with thin crosshair lines
        // through it, like CS 1.6's AWP scope.
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
        h.hurt = s.ui(None, UiNode { absolute: true, width: percent(100.0), height: percent(100.0), ..node() });
        h.flash = s.ui(None, UiNode { absolute: true, width: percent(100.0), height: percent(100.0), ..node() });
        h.ammo = s.ui(None, UiNode { absolute: true, right: px(20.0), bottom: px(14.0), ..big("") });

        // CS 1.6 buy menu: dark panel, amber title, numbered entries, unaffordable ones grayed out.
        h.buy_menu = s.ui(None, UiNode { column: true, gap: 6.0, ..panel() });
        h.buy_title = s.ui(Some(h.buy_menu), text("", 20.0, HUD_AMBER));
        h.buy_rows = (0..8).map(|_| s.ui(Some(h.buy_menu), text("", 18.0, Color::WHITE))).collect();
        s.ui(Some(h.buy_menu), text("B / Esc  close", 13.0, Color::srgb(0.55, 0.55, 0.55)));
        let feed = UiNode { absolute: true, right: px(16.0), top: px(12.0), column: true, align: Align::End, gap: 3.0, ..node() };
        h.kill_feed = s.ui(None, feed);
    }

    pub(super) fn update_hud(&mut self) {
        let alive = self.hp > 0 && self.me.is_some();
        let status = if self.me.is_some() && self.hp <= 0 { "You are dead - spectating" } else { "" };
        let h = &self.hud;
        let s = &mut self.scene;
        s.ui_text(h.status, status);
        s.ui_text(h.health, if alive { self.hp.to_string() } else { String::new() });
        s.ui_text(h.armor, if alive { self.armor.to_string() } else { String::new() });
        s.ui_text(h.money, format!("$ {}", self.money));
        s.ui_text(h.ammo, match self.pred.ammo() {
            Some((mag, reserve)) if self.hp > 0 => format!("{mag}  |  {reserve}"),
            _ => String::new(),
        });
        self.update_round_text();
        self.update_crosshair();
        self.update_menus();
        self.update_kill_feed();
        self.update_score_panel();
        self.update_target();

        let (h, s, time) = (&self.hud, &mut self.scene, self.time);
        // Fade to black, hold, then fade back in on a teammate's view.
        let fade = self.died_at.map_or(0.0, |t| match (time - t) as f32 {
            t if t < 0.8 => t / 0.8,
            t if t < 1.6 => 1.0,
            t => (1.0 - (t - 1.6) / 0.8).max(0.0),
        });
        s.ui_set(h.death_fade, |n| n.background = Color::BLACK.with_alpha(fade));
        let hurt = self.hurt_at.map_or(0.0, |t| (0.35 * (1.0 - (time - t) as f32 / 0.4)).max(0.0));
        s.ui_set(h.hurt, |n| n.background = Color::srgba(0.8, 0.0, 0.0, hurt));
        // Flashed: white for the first part, then fading back.
        let flash = self.flashed.map_or(0.0, |(at, secs)| {
            let k = (time - at) as f32 / secs;
            if k < 0.4 { 1.0 } else { (1.0 - (k - 0.4) / 0.6).max(0.0) }
        });
        s.ui_set(h.flash, |n| n.background = Color::WHITE.with_alpha(flash));
        s.ui_hidden(h.scope, !(self.zoom && self.hp > 0));
        s.ui_text(h.announce, match &self.announce {
            Some((line, at)) if time - at < 3.5 => line.clone(),
            _ => String::new(),
        });
        self.chat.retain(|(_, at)| time - at < 7.0);
        s.ui_text(h.chat, self.chat.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join("\n"));
    }

    fn update_round_text(&mut self) {
        let Some(r) = self.round else { return };
        let secs = r.time_left.ceil() as u32;
        let my_team = self.my_team();
        let message = match (r.phase, r.bomb, r.progress) {
            (Phase::End { winner }, Bomb::Exploded(_), _) => format!("{} win - the bomb exploded", winner.name()),
            (Phase::End { winner }, Bomb::Defused(_), _) => format!("{} win - the bomb was defused", winner.name()),
            (Phase::End { winner }, ..) => format!("{} win", winner.name()),
            (Phase::Freeze, ..) => format!("Round {} - get ready", r.number),
            (_, _, Some((id, p))) if Some(id) == self.me => {
                let verb = if matches!(r.bomb, Bomb::Planted(_)) { "Defusing" } else { "Planting" };
                let bar = "#".repeat((p * 20.0) as usize) + &".".repeat(20 - (p * 20.0) as usize);
                format!("{verb} [{bar}]")
            }
            (_, Bomb::Planted(_), Some(_)) => "BOMB PLANTED - being defused!".into(),
            (_, Bomb::Planted(_), None) => "BOMB PLANTED".into(),
            (_, Bomb::Carried(id), _) if Some(id) == self.me => "You have the bomb - hold E on A or B to plant".into(),
            (_, Bomb::Dropped(_), _) if my_team == Some(Team::T) => "The bomb was dropped - pick it up".into(),
            _ => String::new(),
        };
        let side = my_team.map_or("", |t| t.name());
        self.scene.ui_text(self.hud.timer, format!("{}:{:02}", secs / 60, secs % 60));
        self.scene.ui_text(self.hud.scoreboard, format!("T {}  -  {} CT   ({side})\n{message}", r.score[0], r.score[1]));
    }

    /// CS-style dynamic crosshair: the four lines spread with movement, jumping and recoil, like the real spread.
    /// Grayed out while reloading or drawing, so a held trigger firing late doesn't read as lag. Hidden when scoped.
    fn update_crosshair(&mut self) {
        let p = &self.pred;
        let busy = p.hands.reloading > 0 || p.hands.cooldown as f32 > p.weapon().ability().cycle * TICK_HZ as f32;
        let speed = (p.vel.with_y(0.0).length() / 6.35).min(1.0);
        let air = if p.on_ground { 0.0 } else { 1.0 };
        let gap = 4.0 + 18.0 * speed + 14.0 * air + 3.0 * p.hands.bloom.min(6.0);
        for (id, dir) in self.hud.crosshair {
            // Lines are 9 px long and 2 px thick; offset each from the center by the gap.
            let at = dir * (gap + 4.5);
            let (w, h) = if dir.x != 0.0 { (9.0, 2.0) } else { (2.0, 9.0) };
            self.scene.ui_set(id, |n| {
                (n.width, n.height) = (px(w), px(h));
                (n.left, n.top) = (px(at.x - w / 2.0), px(-at.y - h / 2.0));
                n.background = if busy { Color::srgba(0.5, 0.5, 0.5, 0.6) } else { CROSSHAIR_GREEN };
                n.hidden = self.zoom;
            });
        }
    }

    fn update_menus(&mut self) {
        let debug = |var| std::env::var_os(var).is_some();
        self.scene.ui_hidden(self.hud.radio_menu, !(self.radio_menu || debug("FPS16_RADIOMENU")));
        self.scene.ui_hidden(self.hud.buy_menu, !(self.menu || debug("FPS16_BUYMENU")));
        let (Some(r), Some(team)) = (self.round, self.my_team()) else { return };
        self.scene.ui_text(self.hud.buy_title, format!("BUY   ${}   {:.0}s", self.money, r.buy_secs.ceil()));
        for (i, item) in buy_menu(team).into_iter().enumerate() {
            let money = self.money;
            self.scene.ui_set(self.hud.buy_rows[i], |n| {
                if let Some(t) = &mut n.text {
                    t.value = format!("{}  {:<17}${}", i + 1, item.name(), item.price());
                    t.color = if item.price() <= money { Color::WHITE } else { Color::srgb(0.5, 0.5, 0.5) };
                }
            });
        }
    }

    /// CS 1.6 kill feed: one dark strip per kill, names in team colors, the weapon between, headshots marked.
    fn update_kill_feed(&mut self) {
        let before = self.kill_feed.len();
        let time = self.time;
        self.kill_feed.retain(|l| time - l.at < 6.0);
        if self.kill_feed.len() == before && !self.feed_dirty {
            return;
        }
        self.feed_dirty = false;
        for row in std::mem::take(&mut self.hud.feed_rows) {
            self.scene.ui_despawn(row);
        }
        for line in &self.kill_feed {
            let weapon = format!("  [{}]{}  ", line.weapon.spec().name, if line.headshot { " HS" } else { "" });
            let parts = [(line.killer.0.clone(), team_text_color(line.killer.1)), (weapon, HUD_AMBER), (line.victim.0.clone(), team_text_color(line.victim.1))];
            let row = UiNode { padding: Edges::axes(8.0, 3.0), background: Color::srgba(0.0, 0.0, 0.0, 0.55), ..node() };
            let row = self.scene.ui(Some(self.hud.kill_feed), row);
            for (value, color) in parts {
                let child = self.scene.ui(Some(row), text(&value, 16.0, color));
                // Removed along with the row.
                self.scene.forget(child);
            }
            self.hud.feed_rows.push(row);
        }
    }

    /// CS 1.6 scoreboard while Tab is held: per team, sorted by kills, with deaths and latency; dead players, bots and
    /// the bomb carrier marked.
    fn update_score_panel(&mut self) {
        let show = self.tab || std::env::var_os("FPS16_SCOREBOARD").is_some();
        self.scene.ui_hidden(self.hud.score_panel, !show);
        if !show {
            return;
        }
        let carrier = match self.round.map(|r| r.bomb) {
            Some(Bomb::Carried(id)) => Some(id),
            _ => None,
        };
        for (team, section) in [Team::T, Team::Ct].into_iter().zip(self.hud.sections) {
            let score = self.round.map_or(0, |r| r.score[team as usize]);
            let mut rows: Vec<&NetPlayer> = self.players.iter().filter(|p| p.team == team).collect();
            rows.sort_by_key(|p| (std::cmp::Reverse(p.kills), p.deaths));
            let mut out = format!("{:<34}{:>6}{:>7}{:>9}\n", format!("{}  -  {score}", team.name()), "Score", "Deaths", "Latency");
            for p in rows {
                let tag = match (p.hp <= 0, Some(p.id) == carrier) {
                    (true, _) => "Dead",
                    (_, true) => "Bomb",
                    _ => "",
                };
                let you = if Some(p.id) == self.me { ">" } else { " " };
                let ping = if p.bot { "BOT".to_string() } else { p.ping.to_string() };
                out += &format!("{you}{:<20}{tag:<13}{:>6}{:>7}{:>9}\n", p.name, p.kills, p.deaths, ping);
            }
            self.scene.ui_text(section, out);
        }
    }

    /// "Friend: name  Health: 100%" / "Enemy: name" under the crosshair target, or who we're spectating.
    fn update_target(&mut self) {
        let my_team = self.my_team();
        let player = |id: u32| self.players.iter().find(|p| p.id == id);
        let (line, team) = match (self.hp > 0, self.aim_target, self.spectate) {
            (true, Some(id), _) => match player(id) {
                Some(p) if Some(p.team) == my_team => (format!("Friend: {}   Health: {}%", p.name, p.hp), Some(p.team)),
                Some(p) => (format!("Enemy: {}", p.name), Some(p.team)),
                None => (String::new(), None),
            },
            (false, _, Some((id, ..))) if self.spectating() => (format!("Spectating {}", self.name(id)), my_team),
            _ => (String::new(), None),
        };
        let color = team_text_color(team);
        self.scene.ui_set(self.hud.target, |n| {
            if let Some(t) = &mut n.text {
                (t.value, t.color) = (line, color);
            }
        });
    }
}
