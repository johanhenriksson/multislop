//! Weapon stats (CS 1.6 values converted to meters and seconds), the buy menu and the economy rules.

use crate::shared::Team;
use sdk::ability::{Ability, Accuracy, Cost, Delivery, Payload, Trigger};
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
    He,
    Rocket,
    Flash,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Slot {
    Primary,
    #[default]
    Secondary,
    Knife,
    Grenade,
}

/// A weapon: what using it does (`ability`) plus what this game needs around it.
pub struct Spec {
    pub name: &'static str,
    pub price: u32,
    /// Only this team can buy it.
    pub team: Option<Team>,
    pub slot: Slot,
    pub kill_reward: u32,
    /// Movement speed multiplier while this weapon is in hand.
    pub speed: f32,
    pub ability: Ability,
}

const ACCURACY: Accuracy = Accuracy { spread: 0.01, moving: 0.04, airborne: 0.12, kick: 0.01 };

const fn gun(name: &'static str, price: u32, team: Option<Team>, slot: Slot, ability: Ability) -> Spec {
    Spec { name, price, team, slot, kill_reward: 300, speed: 1.0, ability }
}

/// A hitscan gun firing `damage`, with `range_mod` the damage kept per 12.7 m and `pen` the part that gets through armor.
#[allow(clippy::too_many_arguments)]
const fn hitscan(auto: bool, cycle: f32, (mag, reserve, reload): (u8, u16, f32), accuracy: Accuracy, damage: f32, range_mod: f32, pen: f32) -> Ability {
    Ability {
        trigger: if auto { Trigger::Auto } else { Trigger::Semi },
        cost: Cost::Ammo { mag, reserve, reload },
        cycle,
        accuracy,
        delivery: Delivery::Hitscan { range: 200.0, falloff: range_mod },
        payload: Payload::Damage { amount: damage, armor_pen: pen },
    }
}

const KNIFE: Spec = Spec {
    kill_reward: 1500,
    speed: 1.08,
    ..gun(
        "Knife",
        0,
        None,
        Slot::Knife,
        Ability {
            trigger: Trigger::Auto,
            cost: Cost::Free,
            cycle: 0.4,
            accuracy: Accuracy { spread: 0.0, moving: 0.0, kick: 0.0, ..ACCURACY },
            delivery: Delivery::Melee { reach: 1.8 },
            payload: Payload::Damage { amount: 35.0, armor_pen: 0.85 },
        },
    )
};
const GLOCK: Spec = gun(
    "Glock-18",
    400,
    Some(Team::T),
    Slot::Secondary,
    hitscan(false, 0.15, (20, 120, 2.2), Accuracy { spread: 0.012, moving: 0.03, kick: 0.012, ..ACCURACY }, 25.0, 0.75, 0.5),
);
const USP: Spec = gun(
    "USP",
    500,
    Some(Team::Ct),
    Slot::Secondary,
    hitscan(false, 0.15, (12, 100, 2.7), Accuracy { spread: 0.01, moving: 0.03, kick: 0.015, ..ACCURACY }, 34.0, 0.79, 0.5),
);
const DEAGLE: Spec = gun(
    "Desert Eagle",
    650,
    None,
    Slot::Secondary,
    hitscan(false, 0.225, (7, 35, 2.2), Accuracy { spread: 0.008, moving: 0.06, kick: 0.045, ..ACCURACY }, 54.0, 0.81, 0.75),
);
const MP5: Spec = Spec {
    kill_reward: 600,
    ..gun(
        "MP5",
        1500,
        None,
        Slot::Primary,
        hitscan(true, 0.075, (30, 120, 2.6), Accuracy { spread: 0.012, moving: 0.02, kick: 0.006, ..ACCURACY }, 26.0, 0.84, 0.6),
    )
};
const AK47: Spec = Spec {
    speed: 0.88,
    ..gun(
        "AK-47",
        2500,
        Some(Team::T),
        Slot::Primary,
        hitscan(true, 0.0955, (30, 90, 2.4), Accuracy { spread: 0.006, moving: 0.06, kick: 0.012, ..ACCURACY }, 36.0, 0.98, 0.775),
    )
};
const M4A1: Spec = Spec {
    speed: 0.92,
    ..gun(
        "M4A1",
        3100,
        Some(Team::Ct),
        Slot::Primary,
        hitscan(true, 0.0875, (30, 90, 3.0), Accuracy { spread: 0.005, moving: 0.05, kick: 0.01, ..ACCURACY }, 33.0, 0.97, 0.7),
    )
};
const AWP: Spec = Spec {
    kill_reward: 100,
    speed: 0.84,
    // Unscoped spread; scoped is AWP_SCOPED_SPREAD.
    ..gun(
        "AWP",
        4750,
        None,
        Slot::Primary,
        hitscan(false, 1.45, (10, 30, 3.0), Accuracy { spread: 0.06, moving: 0.1, kick: 0.03, ..ACCURACY }, 115.0, 0.99, 0.97),
    )
};
/// Hold fire to wind up, release to throw; harder the longer it was held. CS 1.6 HE damage and radius.
const HE: Spec = gun(
    "HE Grenade",
    300,
    None,
    Slot::Grenade,
    Ability {
        trigger: Trigger::Charge { full: 0.6 },
        cost: Cost::Ammo { mag: 1, reserve: 0, reload: 0.0 },
        cycle: 0.5,
        accuracy: Accuracy { spread: 0.0, moving: 0.0, airborne: 0.0, kick: 0.0 },
        delivery: Delivery::Projectile { speed: 25.0, gravity: 20.3, bounce: 0.45, fuse: 2.0, contact: false },
        payload: Payload::Explode { damage: 98.0, radius: 8.9, armor_pen: 0.5 },
    },
);
/// Thrown like the HE, but goes off in a blinding flash. Two can be carried, bought one at a time.
const FLASH: Spec = gun(
    "Flashbang",
    200,
    None,
    Slot::Grenade,
    Ability {
        trigger: Trigger::Charge { full: 0.6 },
        cost: Cost::Ammo { mag: 2, reserve: 0, reload: 0.0 },
        cycle: 0.5,
        accuracy: Accuracy { spread: 0.0, moving: 0.0, airborne: 0.0, kick: 0.0 },
        delivery: Delivery::Projectile { speed: 25.0, gravity: 20.3, bounce: 0.45, fuse: 1.5, contact: false },
        payload: Payload::Flash { radius: 25.0, secs: 4.0 },
    },
);
/// A rocket launcher: four rockets to a load, flying straight and fast and going off on whatever they touch.
const ROCKET: Spec = Spec {
    speed: 0.75,
    ..gun(
        "Rocket Launcher",
        13000,
        None,
        Slot::Primary,
        Ability {
            trigger: Trigger::Semi,
            cost: Cost::Ammo { mag: 4, reserve: 16, reload: 3.0 },
            cycle: 0.7,
            accuracy: Accuracy { spread: 0.0, moving: 0.01, airborne: 0.03, kick: 0.08 },
            delivery: Delivery::Projectile { speed: 35.0, gravity: 0.0, bounce: 0.0, fuse: 6.0, contact: true },
            payload: Payload::Explode { damage: 250.0, radius: 7.0, armor_pen: 0.8 },
        },
    )
};

pub const ALL_WEAPONS: [Weapon; 11] = [
    Weapon::Knife,
    Weapon::Glock,
    Weapon::Usp,
    Weapon::Deagle,
    Weapon::Mp5,
    Weapon::Ak47,
    Weapon::M4a1,
    Weapon::Awp,
    Weapon::He,
    Weapon::Rocket,
    Weapon::Flash,
];

pub const AWP_SCOPED_SPREAD: f32 = 0.0005;
/// Moving while scoped in is slower still.
pub const SCOPED_SPEED: f32 = 0.6;
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
            Weapon::He => &HE,
            Weapon::Rocket => &ROCKET,
            Weapon::Flash => &FLASH,
        }
    }

    pub fn ability(self) -> &'static Ability {
        &self.spec().ability
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
pub fn buy_menu(team: Team) -> [Buy; 8] {
    let rifle = if team == Team::T { Weapon::Ak47 } else { Weapon::M4a1 };
    let guns = [Weapon::Deagle, Weapon::Mp5, rifle, Weapon::Awp].map(Buy::Weapon);
    let more = [Weapon::He, Weapon::Flash, Weapon::Rocket].map(Buy::Weapon);
    [guns[0], guns[1], guns[2], guns[3], Buy::Armor, more[0], more[1], more[2]]
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
