//! The HUD: the race clock (or countdown), place and checkpoint, finishes in the feed, big centered messages, and
//! the standings (Tab, and once the race is over).

use super::{RaceClient, ordinal};
use crate::map::course;
use crate::shared::{NetPlayer, Phase};
use sdk::client::*;

const TEXT: Color = Color::srgba(1.0, 1.0, 1.0, 0.92);
const ACCENT: Color = Color::srgba(1.0, 0.8, 0.2, 0.95);

/// HUD node ids.
#[derive(Default)]
pub struct Hud {
    clock: u32,
    closing: u32,
    place: u32,
    progress: u32,
    feed: u32,
    message: u32,
    board: u32,
    board_text: u32,
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

/// A full-width strip at `top` percent down the screen, children centered.
fn strip(top: f32) -> UiNode {
    UiNode { absolute: true, top: percent(top), width: percent(100.0), justify: Align::Center, ..node() }
}

fn clock(secs: f32) -> String {
    format!("{}:{:05.2}", (secs / 60.0) as u32, secs % 60.0)
}

impl RaceClient {
    pub(super) fn build_hud(&mut self) {
        let s = &mut self.scene;
        let h = &mut self.hud;
        let dot = UiNode { absolute: true, left: percent(50.0), top: percent(50.0), width: px(4.0), height: px(4.0), margin: Edges { left: -2.0, top: -2.0, ..Default::default() }, radius: Corners::all(2.0), background: TEXT, ..node() };
        s.ui(None, dot);

        let top = s.ui(None, UiNode { column: true, align: Align::Center, ..strip(1.5) });
        h.clock = s.ui(Some(top), centered(30.0, TEXT));
        h.closing = s.ui(Some(top), centered(18.0, Color::srgb(1.0, 0.45, 0.35)));
        let corner = s.ui(None, UiNode { absolute: true, left: px(24.0), bottom: px(16.0), column: true, ..node() });
        h.place = s.ui(Some(corner), text(44.0, ACCENT));
        h.progress = s.ui(Some(corner), text(18.0, TEXT));
        let feed = s.ui(None, UiNode { absolute: true, right: px(20.0), top: px(14.0), ..node() });
        h.feed = s.ui(Some(feed), text(16.0, TEXT));
        let middle = s.ui(None, strip(28.0));
        h.message = s.ui(Some(middle), centered(32.0, ACCENT));
        let hint = s.ui(None, UiNode { absolute: true, bottom: px(14.0), width: percent(100.0), justify: Align::Center, ..node() });
        let hint = s.ui(Some(hint), centered(14.0, Color::srgba(1.0, 1.0, 1.0, 0.5)));
        s.ui_text(hint, "R: back to checkpoint   Tab: standings");

        let board = UiNode { hidden: true, absolute: true, width: percent(100.0), height: percent(100.0), justify: Align::Center, align: Align::Center, ..node() };
        h.board = s.ui(None, board);
        let panel = UiNode { padding: Edges::all(20.0), radius: Corners::all(6.0), background: Color::srgba(0.0, 0.0, 0.05, 0.8), ..node() };
        let panel = s.ui(Some(h.board), panel);
        h.board_text = s.ui(Some(panel), text(18.0, TEXT));
    }

    pub(super) fn update_hud(&mut self) {
        let mut ranked: Vec<&NetPlayer> = self.players.iter().collect();
        ranked.sort_by_key(|p| p.standing());
        let mine = ranked.iter().position(|p| Some(p.id) == self.me).map(|i| (i, ranked[i]));

        let (clock_text, closing) = match self.race {
            Some(r) if r.phase == Phase::Countdown => (format!("Starting in {}", r.time.ceil() as u32), String::new()),
            Some(r) if r.phase == Phase::Results => (format!("Next race in {}", r.time.ceil() as u32), String::new()),
            Some(r) => {
                // Our own time stops when we finish.
                let shown = mine.and_then(|(_, p)| p.finish).map_or(r.time, |(_, secs)| secs);
                (clock(shown), r.closing.map_or(String::new(), |c| format!("{} s left to finish", c.ceil() as u32)))
            }
            None => (String::new(), String::new()),
        };
        self.scene.ui_text(self.hud.clock, clock_text);
        self.scene.ui_text(self.hud.closing, closing);

        let (place, progress) = mine.map_or((String::new(), String::new()), |(i, p)| match p.finish {
            Some((place, _)) => (ordinal(place), "Finished".into()),
            None => (format!("{} of {}", ordinal(i as u8 + 1), ranked.len()), format!("Checkpoint {} of {}", p.checkpoint, course().checkpoints())),
        });
        self.scene.ui_text(self.hud.place, place);
        self.scene.ui_text(self.hud.progress, progress);

        while self.feed.front().is_some_and(|(_, at)| self.time - at > 6.0) {
            self.feed.pop_front();
        }
        let feed: Vec<&str> = self.feed.iter().map(|(line, _)| line.as_str()).collect();
        self.scene.ui_text(self.hud.feed, feed.join("\n"));

        let results = self.race.is_some_and(|r| r.phase == Phase::Results);
        let message = match (&self.message, ranked.first()) {
            (_, Some(w)) if results && w.finish.is_some() && Some(w.id) == self.me => "You win!".to_string(),
            (_, Some(w)) if results && w.finish.is_some() => format!("{} wins!", w.name),
            (_, _) if results => "Nobody made it".into(),
            (Some((m, at)), _) if self.time - at < 2.0 => m.clone(),
            _ => String::new(),
        };
        self.scene.ui_text(self.hud.message, message);

        let show = self.tab || results;
        self.scene.ui_hidden(self.hud.board, !show);
        if show {
            let mut out = format!("  {:<4}{:<20}{:>10}{:>6}{:>7}\n", "", "Runner", "Time", "Wins", "Ping");
            for (i, p) in ranked.iter().enumerate() {
                let you = if Some(p.id) == self.me { ">" } else { " " };
                let time = p.finish.map_or_else(|| format!("CP {}", p.checkpoint), |(_, secs)| clock(secs));
                let ping = if p.bot { "BOT".into() } else { p.ping.to_string() };
                out += &format!("{you} {:<4}{:<20}{:>10}{:>6}{:>7}\n", ordinal(i as u8 + 1), p.name, time, p.wins, ping);
            }
            self.scene.ui_text(self.hud.board_text, out);
        }
    }
}
