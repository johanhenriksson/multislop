//! The HUD: hole, par and strokes, the hole clock, the power bar while pulling back, holes made in the feed, big
//! centered messages, and the scorecard (Tab, and between holes).

use super::{FULL_PULL, GolfClient, readable, rgb};
use crate::map::{HOLES, course};
use crate::shared::{NetPlayer, Phase};
use sdk::client::*;

const TEXT: Color = Color::srgba(1.0, 1.0, 1.0, 0.92);
const ACCENT: Color = Color::srgba(1.0, 0.85, 0.3, 0.95);

/// HUD node ids.
#[derive(Default)]
pub struct Hud {
    hole: u32,
    strokes: u32,
    clock: u32,
    feed: u32,
    message: u32,
    hint: u32,
    power: u32,
    power_fill: u32,
    card: u32,
    card_panel: u32,
    pars: u32,
    /// Scorecard lines: a swatch of the player's color and the line, in it. Made as more players show up.
    card_rows: Vec<(u32, u32, u32)>,
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

/// Strokes against par: "E", "+3", "-1".
fn relative(diff: i32) -> String {
    match diff {
        0 => "E".into(),
        d if d > 0 => format!("+{d}"),
        d => d.to_string(),
    }
}

impl GolfClient {
    pub(super) fn build_hud(&mut self) {
        let s = &mut self.scene;
        let h = &mut self.hud;
        let corner = s.ui(None, UiNode { absolute: true, left: px(24.0), top: px(16.0), column: true, ..node() });
        h.hole = s.ui(Some(corner), text(30.0, ACCENT));
        h.strokes = s.ui(Some(corner), text(20.0, TEXT));
        let top = s.ui(None, strip(1.5));
        h.clock = s.ui(Some(top), centered(28.0, TEXT));
        let feed = s.ui(None, UiNode { absolute: true, right: px(20.0), top: px(14.0), ..node() });
        h.feed = s.ui(Some(feed), text(16.0, TEXT));
        let middle = s.ui(None, strip(26.0));
        h.message = s.ui(Some(middle), centered(34.0, ACCENT));

        let bar = UiNode { absolute: true, bottom: px(60.0), width: percent(100.0), justify: Align::Center, ..node() };
        let bar = s.ui(None, bar);
        let track = UiNode { width: px(320.0), height: px(16.0), radius: Corners::all(8.0), background: Color::srgba(0.0, 0.0, 0.0, 0.5), border: Edges::all(2.0), border_color: TEXT, hidden: true, ..node() };
        h.power = s.ui(Some(bar), track);
        h.power_fill = s.ui(Some(h.power), UiNode { width: percent(0.0), height: percent(100.0), radius: Corners::all(6.0), background: ACCENT, ..node() });

        let hint = s.ui(None, UiNode { absolute: true, bottom: px(14.0), width: percent(100.0), justify: Align::Center, ..node() });
        h.hint = s.ui(Some(hint), centered(14.0, Color::srgba(1.0, 1.0, 1.0, 0.55)));

        let card = UiNode { hidden: true, absolute: true, width: percent(100.0), height: percent(100.0), justify: Align::Center, align: Align::Center, ..node() };
        h.card = s.ui(None, card);
        let panel = UiNode { padding: Edges::all(20.0), radius: Corners::all(6.0), background: Color::srgba(0.0, 0.0, 0.05, 0.8), column: true, gap: 4.0, ..node() };
        h.card_panel = s.ui(Some(h.card), panel);
        let header = s.ui(Some(h.card_panel), text(18.0, TEXT));
        let holes: String = (1..=HOLES).map(|n| format!("{n:>3}")).collect();
        s.ui_text(header, format!("     {:<16}{holes}{:>6}{:>6}{:>6}", "Player", "Tot", "", "Ping"));
        h.pars = s.ui(Some(h.card_panel), text(18.0, Color::srgba(1.0, 1.0, 1.0, 0.55)));
        let pars: String = course().holes.iter().map(|h| format!("{:>3}", h.par)).collect();
        let total: u32 = course().holes.iter().map(|h| h.par as u32).sum();
        s.ui_text(h.pars, format!("     {:<16}{pars}{total:>6}", "Par"));
    }

    pub(super) fn update_hud(&mut self) {
        let Some(round) = self.round else { return };
        let hole = &course().holes[round.hole as usize];
        self.scene.ui_text(self.hud.hole, format!("Hole {} of {HOLES}", round.hole + 1));
        self.scene.ui_text(self.hud.strokes, format!("Par {}   Strokes {}", hole.par, self.pred.strokes));

        let clock = match round.phase {
            Phase::Tee => String::new(),
            Phase::Play => format!("{}:{:02}", round.time as u32 / 60, round.time as u32 % 60),
            Phase::Card => format!("Next hole in {}", round.time.ceil() as u32),
            Phase::Final => format!("New course in {}", round.time.ceil() as u32),
        };
        self.scene.ui_text(self.hud.clock, clock);

        while self.feed.front().is_some_and(|(_, at)| self.time - at > 6.0) {
            self.feed.pop_front();
        }
        let feed: Vec<&str> = self.feed.iter().map(|(line, _)| line.as_str()).collect();
        self.scene.ui_text(self.hud.feed, feed.join("\n"));

        let mut ranked: Vec<&NetPlayer> = self.players.iter().collect();
        ranked.sort_by_key(|p| p.total());
        let message = match (round.phase, &self.message) {
            (Phase::Tee, _) => format!("Hole {}\nPar {}", round.hole + 1, hole.par),
            (Phase::Final, _) => match ranked.first() {
                Some(w) if Some(w.id) == self.me => "You win!".into(),
                Some(w) => format!("{} wins!", w.name),
                None => String::new(),
            },
            (_, Some((m, at))) if self.time - at < 2.5 => m.clone(),
            _ => String::new(),
        };
        self.scene.ui_text(self.hud.message, message);

        let done = self.done();
        let hint = match round.phase {
            Phase::Play if done && !self.playing().is_empty() => "Click: watch the next player   Tab: scorecard",
            Phase::Play if done => "Waiting for the hole to end   Tab: scorecard",
            Phase::Play => "Hold left click and pull back to putt   Right click: cancel   W/S: zoom   Tab: scorecard",
            _ => "",
        };
        self.scene.ui_text(self.hud.hint, hint);

        let power = self.pull.map(|p| (p / FULL_PULL).min(1.0));
        self.scene.ui_hidden(self.hud.power, power.is_none());
        let fill = Color::srgb(1.0, 0.85 - power.unwrap_or(0.0) * 0.65, 0.3 - power.unwrap_or(0.0) * 0.2);
        self.scene.ui_set(self.hud.power_fill, |n| {
            n.width = percent(power.unwrap_or(0.0) * 100.0);
            n.background = fill;
        });

        let show = self.tab || matches!(round.phase, Phase::Card | Phase::Final);
        self.scene.ui_hidden(self.hud.card, !show);
        if !show {
            return;
        }
        let pars: Vec<u8> = course().holes.iter().map(|h| h.par).collect();
        let s = &mut self.scene;
        while self.hud.card_rows.len() < ranked.len() {
            let row = s.ui(Some(self.hud.card_panel), UiNode { align: Align::Center, gap: 10.0, ..node() });
            let swatch = s.ui(Some(row), UiNode { width: px(14.0), height: px(14.0), radius: Corners::all(3.0), ..node() });
            self.hud.card_rows.push((row, swatch, s.ui(Some(row), text(18.0, TEXT))));
        }
        for (i, &(row, swatch, line)) in self.hud.card_rows.iter().enumerate() {
            s.ui_hidden(row, i >= ranked.len());
            let Some(p) = ranked.get(i) else { continue };
            let you = if Some(p.id) == self.me { ">" } else { " " };
            let holes: String = p.card.iter().map(|&n| if n == 0 { "  -".into() } else { format!("{n:>3}") }).collect();
            let par: i32 = p.card.iter().zip(&pars).filter(|(n, _)| **n > 0).map(|(_, &par)| par as i32).sum();
            let ping = if p.bot { "BOT".into() } else { p.ping.to_string() };
            let value = format!("{you}{:<16}{holes}{:>6}{:>6}{ping:>6}", p.name, p.total(), relative(p.total() as i32 - par));
            let (color, text) = (rgb(p.color), readable(p.color));
            s.ui_set(swatch, |n| n.background = color);
            s.ui_set(line, |n| n.text = Some(Text { value, color: text, shadow: true, center: false, size: 18.0 }));
        }
    }
}
