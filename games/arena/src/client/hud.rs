//! The HUD: crosshair with the rail's recharge under it, frags and rank, match clock, kill feed, frag messages,
//! the respawn prompt and the scoreboard (Tab, and at the end of a match).

use super::ArenaClient;
use sdk::client::*;

const TEXT: Color = Color::srgba(1.0, 1.0, 1.0, 0.92);
const ACCENT: Color = Color::srgba(0.3, 0.9, 1.0, 0.95);

/// HUD node ids.
#[derive(Default)]
pub struct Hud {
    charge: u32,
    frags: u32,
    rank: u32,
    clock: u32,
    /// Kill feed lines, and their killer, " railed ", victim texts.
    feed: Vec<(u32, [u32; 3])>,
    message: u32,
    dead: u32,
    board: u32,
    board_panel: u32,
    /// Scoreboard lines: a swatch of the player's color and the line, in it. Made as more players show up.
    board_rows: Vec<(u32, u32, u32)>,
}

/// Most kill feed lines.
const FEED: usize = 5;

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

impl ArenaClient {
    pub(super) fn build_hud(&mut self) {
        let s = &mut self.scene;
        let h = &mut self.hud;
        // Crosshair: a dot and four ticks around the exact center, with the recharge bar under it.
        let layer = s.ui(None, screen());
        let anchor = s.ui(Some(layer), UiNode { width: px(0.0), height: px(0.0), ..node() });
        let tick = |left: f32, top: f32, w: f32, h: f32| UiNode { absolute: true, left: px(left), top: px(top), width: px(w), height: px(h), background: TEXT, ..node() };
        for t in [tick(-1.0, -1.0, 2.0, 2.0), tick(-12.0, -1.0, 7.0, 2.0), tick(5.0, -1.0, 7.0, 2.0), tick(-1.0, -12.0, 2.0, 7.0), tick(-1.0, 5.0, 2.0, 7.0)] {
            s.ui(Some(anchor), t);
        }
        let track = UiNode { absolute: true, left: px(-20.0), top: px(18.0), width: px(40.0), height: px(3.0), background: Color::srgba(1.0, 1.0, 1.0, 0.15), ..node() };
        let track = s.ui(Some(anchor), track);
        h.charge = s.ui(Some(track), UiNode { height: percent(100.0), width: percent(100.0), background: ACCENT, ..node() });

        let corner = s.ui(None, UiNode { absolute: true, left: px(24.0), bottom: px(16.0), column: true, ..node() });
        h.frags = s.ui(Some(corner), text(44.0, ACCENT));
        h.rank = s.ui(Some(corner), text(18.0, TEXT));
        let top = s.ui(None, UiNode { absolute: true, top: px(12.0), width: percent(100.0), justify: Align::Center, ..node() });
        h.clock = s.ui(Some(top), centered(22.0, TEXT));
        let feed = s.ui(None, UiNode { absolute: true, right: px(20.0), top: px(14.0), column: true, align: Align::End, gap: 2.0, ..node() });
        for _ in 0..FEED {
            let line = s.ui(Some(feed), UiNode { hidden: true, gap: 6.0, ..node() });
            h.feed.push((line, [0; 3].map(|_| s.ui(Some(line), text(16.0, TEXT)))));
        }
        let middle = s.ui(None, UiNode { absolute: true, top: percent(30.0), width: percent(100.0), justify: Align::Center, ..node() });
        h.message = s.ui(Some(middle), centered(26.0, Color::srgb(1.0, 0.85, 0.3)));
        let lower = s.ui(None, UiNode { absolute: true, top: percent(60.0), width: percent(100.0), justify: Align::Center, ..node() });
        h.dead = s.ui(Some(lower), centered(24.0, TEXT));

        let board = UiNode { hidden: true, ..screen() };
        h.board = s.ui(None, board);
        let panel = UiNode { padding: Edges::all(20.0), radius: Corners::all(6.0), background: Color::srgba(0.0, 0.0, 0.05, 0.8), ..node() };
        h.board_panel = s.ui(Some(h.board), UiNode { column: true, gap: 4.0, ..panel });
        let header = s.ui(Some(h.board_panel), text(18.0, TEXT));
        s.ui_text(header, format!("     {:<22}{:>7}{:>8}{:>9}", "Player", "Frags", "Deaths", "Ping"));
    }

    pub(super) fn update_hud(&mut self) {
        let charge = 1.0 - self.pred.hands.cooldown as f32 / (crate::shared::RAIL.cycle * crate::shared::TICK_HZ as f32);
        self.scene.ui_set(self.hud.charge, |n| n.width = percent((charge * 10.0).round() * 10.0));

        let mut ranked: Vec<&crate::shared::NetPlayer> = self.players.iter().collect();
        ranked.sort_by_key(|p| (std::cmp::Reverse(p.frags), p.deaths));
        let place = ranked.iter().position(|p| Some(p.id) == self.me);
        let mine = place.map_or(0, |i| ranked[i].frags);
        let rank = place.map_or(String::new(), |i| {
            let lead = if i == 0 { ranked.get(1).map_or(mine, |p| mine - p.frags) } else { mine - ranked[0].frags };
            format!("{} of {}   {}{lead}", ordinal(i + 1), ranked.len(), if lead >= 0 { "+" } else { "" })
        });
        self.scene.ui_text(self.hud.frags, mine.to_string());
        self.scene.ui_text(self.hud.rank, rank);

        let clock = self.state.map_or(String::new(), |m| {
            let secs = m.time_left.ceil() as u32;
            match m.winner {
                Some(_) => format!("Next match in {secs}"),
                None => format!("{}:{:02}   first to {}", secs / 60, secs % 60, m.frag_limit),
            }
        });
        self.scene.ui_text(self.hud.clock, clock);

        while self.feed.front().is_some_and(|(_, at)| self.time - at > 6.0) {
            self.feed.pop_front();
        }
        for (i, &(node, [killer, railed, victim])) in self.hud.feed.iter().enumerate() {
            let line = self.feed.get(i);
            self.scene.ui_hidden(node, line.is_none());
            let Some(([(k, kc), (v, vc)], _)) = line else { continue };
            for (id, value, color) in [(killer, k.as_str(), *kc), (railed, "railed", TEXT), (victim, v.as_str(), *vc)] {
                self.scene.ui_set(id, |n| n.text = Some(Text { value: value.into(), color, shadow: true, center: false, size: 16.0 }));
            }
        }

        let winner = self.state.and_then(|m| m.winner);
        let message = match (winner, &self.message) {
            (Some(id), _) if Some(id) == self.me => "You win!".to_string(),
            (Some(id), _) => format!("{} wins!", self.name(id)),
            (None, Some((m, at))) if self.time - at < 2.0 => m.clone(),
            _ => String::new(),
        };
        self.scene.ui_text(self.hud.message, message);
        let dead = match self.killer {
            _ if self.alive || self.me.is_none() || winner.is_some() => String::new(),
            Some(k) if Some(k) != self.me => format!("Fragged by {}\nClick to respawn", self.name(k)),
            _ => "Click to respawn".into(),
        };
        self.scene.ui_text(self.hud.dead, dead);

        let show = self.tab || winner.is_some();
        self.scene.ui_hidden(self.hud.board, !show);
        if show {
            let s = &mut self.scene;
            while self.hud.board_rows.len() < ranked.len() {
                let row = s.ui(Some(self.hud.board_panel), UiNode { align: Align::Center, gap: 10.0, ..node() });
                let swatch = s.ui(Some(row), UiNode { width: px(14.0), height: px(14.0), radius: Corners::all(3.0), ..node() });
                self.hud.board_rows.push((row, swatch, s.ui(Some(row), text(18.0, TEXT))));
            }
            for (i, &(row, swatch, line)) in self.hud.board_rows.iter().enumerate() {
                s.ui_hidden(row, i >= ranked.len());
                let Some(p) = ranked.get(i) else { continue };
                let you = if Some(p.id) == self.me { ">" } else { " " };
                let ping = if p.bot { "BOT".into() } else { p.ping.to_string() };
                let value = format!("{you} {:<22}{:>7}{:>8}{:>9}", p.name, p.frags, p.deaths, ping);
                let (color, text) = (super::rgb(p.color), super::readable(p.color));
                s.ui_set(swatch, |n| n.background = color);
                s.ui_set(line, |n| n.text = Some(Text { value, color: text, shadow: true, center: false, size: 18.0 }));
            }
        }
    }
}

fn ordinal(n: usize) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}
