//! Weapon stats (CS 1.6 values converted to meters and seconds), the buy menu and the economy rules.

use crate::shared::Team;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub enum Weapon {
    #[default]
    Knife,
    Glock,
    Usp,
    Deagle,
    Mp5,
    Ak47,
    M4a1,
    Awp,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Slot {
    Primary,
    #[default]
    Secondary,
    Knife,
}

pub struct Spec {
    pub name: &'static str,
    pub price: u32,
    /// Only this team can buy it.
    pub team: Option<Team>,
    pub slot: Slot,
    pub damage: f32,
    /// Seconds between shots.
    pub cycle: f32,
    pub auto: bool,
    pub mag: u8,
    pub reserve: u16,
    pub reload: f32,
    /// Spread cone half-angle when standing still, radians.
    pub spread: f32,
    /// Extra spread at full running speed.
    pub move_spread: f32,
    /// View kick per shot, radians.
    pub recoil: f32,
    /// Damage multiplier per 12.7 m (500 units) travelled.
    pub range_mod: f32,
    /// Fraction of damage that gets through armor.
    pub armor_pen: f32,
    pub kill_reward: u32,
    pub range: f32,
}

const fn gun(name: &'static str, price: u32, team: Option<Team>, slot: Slot) -> Spec {
    Spec {
        name,
        price,
        team,
        slot,
        damage: 0.0,
        cycle: 0.1,
        auto: false,
        mag: 0,
        reserve: 0,
        reload: 2.5,
        spread: 0.01,
        move_spread: 0.04,
        recoil: 0.01,
        range_mod: 0.9,
        armor_pen: 0.5,
        kill_reward: 300,
        range: 200.0,
    }
}

const KNIFE: Spec = Spec {
    damage: 35.0,
    cycle: 0.4,
    auto: true,
    spread: 0.0,
    move_spread: 0.0,
    recoil: 0.0,
    range_mod: 1.0,
    armor_pen: 0.85,
    kill_reward: 1500,
    range: 1.8,
    ..gun("Knife", 0, None, Slot::Knife)
};
const GLOCK: Spec = Spec {
    damage: 25.0,
    cycle: 0.15,
    mag: 20,
    reserve: 120,
    reload: 2.2,
    spread: 0.012,
    move_spread: 0.03,
    recoil: 0.012,
    range_mod: 0.75,
    ..gun("Glock-18", 400, Some(Team::T), Slot::Secondary)
};
const USP: Spec = Spec {
    damage: 34.0,
    cycle: 0.15,
    mag: 12,
    reserve: 100,
    reload: 2.7,
    spread: 0.01,
    move_spread: 0.03,
    recoil: 0.015,
    range_mod: 0.79,
    ..gun("USP", 500, Some(Team::Ct), Slot::Secondary)
};
const DEAGLE: Spec = Spec {
    damage: 54.0,
    cycle: 0.225,
    mag: 7,
    reserve: 35,
    reload: 2.2,
    spread: 0.008,
    move_spread: 0.06,
    recoil: 0.045,
    range_mod: 0.81,
    armor_pen: 0.75,
    ..gun("Desert Eagle", 650, None, Slot::Secondary)
};
const MP5: Spec = Spec {
    damage: 26.0,
    cycle: 0.075,
    auto: true,
    mag: 30,
    reserve: 120,
    reload: 2.6,
    spread: 0.012,
    move_spread: 0.02,
    recoil: 0.006,
    range_mod: 0.84,
    armor_pen: 0.6,
    kill_reward: 600,
    ..gun("MP5", 1500, None, Slot::Primary)
};
const AK47: Spec = Spec {
    damage: 36.0,
    cycle: 0.0955,
    auto: true,
    mag: 30,
    reserve: 90,
    reload: 2.4,
    spread: 0.006,
    move_spread: 0.06,
    recoil: 0.012,
    range_mod: 0.98,
    armor_pen: 0.775,
    ..gun("AK-47", 2500, Some(Team::T), Slot::Primary)
};
const M4A1: Spec = Spec {
    damage: 33.0,
    cycle: 0.0875,
    auto: true,
    mag: 30,
    reserve: 90,
    reload: 3.0,
    spread: 0.005,
    move_spread: 0.05,
    recoil: 0.01,
    range_mod: 0.97,
    armor_pen: 0.7,
    ..gun("M4A1", 3100, Some(Team::Ct), Slot::Primary)
};
const AWP: Spec = Spec {
    damage: 115.0,
    cycle: 1.45,
    mag: 10,
    reserve: 30,
    reload: 3.0,
    // Unscoped; scoped is AWP_SCOPED_SPREAD.
    spread: 0.06,
    move_spread: 0.1,
    recoil: 0.03,
    range_mod: 0.99,
    armor_pen: 0.97,
    kill_reward: 100,
    ..gun("AWP", 4750, None, Slot::Primary)
};

pub const AWP_SCOPED_SPREAD: f32 = 0.0005;
/// Seconds after switching weapons before you can fire.
pub const DRAW_SECS: f32 = 0.4;

impl Weapon {
    pub fn spec(self) -> &'static Spec {
        match self {
            Weapon::Knife => &KNIFE,
            Weapon::Glock => &GLOCK,
            Weapon::Usp => &USP,
            Weapon::Deagle => &DEAGLE,
            Weapon::Mp5 => &MP5,
            Weapon::Ak47 => &AK47,
            Weapon::M4a1 => &M4A1,
            Weapon::Awp => &AWP,
        }
    }

    pub fn default_pistol(team: Team) -> Weapon {
        match team {
            Team::T => Weapon::Glock,
            Team::Ct => Weapon::Usp,
        }
    }
}

pub const START_MONEY: u32 = 800;
pub const MAX_MONEY: u32 = 16000;
pub const WIN_REWARD: u32 = 3250;
pub const LOSS_REWARD: u32 = 1400;
/// Added to the loss reward per consecutive loss, up to three.
pub const LOSS_BONUS: u32 = 500;
pub const OBJECTIVE_REWARD: u32 = 300;
pub const ARMOR_PRICE: u32 = 1000;
/// Buying is allowed this long into the live round (plus all of freeze time)...
pub const BUY_SECS: f32 = 20.0;
/// ...within this distance of your own spawn.
pub const BUY_RADIUS: f32 = 15.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Buy {
    Weapon(Weapon),
    Armor,
}

/// The buy menu, numbered from 1. The rifle entry depends on the team.
pub fn buy_menu(team: Team) -> [Buy; 5] {
    let rifle = if team == Team::T { Weapon::Ak47 } else { Weapon::M4a1 };
    [Buy::Weapon(Weapon::Deagle), Buy::Weapon(Weapon::Mp5), Buy::Weapon(rifle), Buy::Weapon(Weapon::Awp), Buy::Armor]
}

impl Buy {
    pub fn price(self) -> u32 {
        match self {
            Buy::Weapon(w) => w.spec().price,
            Buy::Armor => ARMOR_PRICE,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Buy::Weapon(w) => w.spec().name,
            Buy::Armor => "Kevlar + Helmet",
        }
    }
}
