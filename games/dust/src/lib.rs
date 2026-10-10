pub mod bot;
pub mod game;
pub mod map;
pub mod shared;
pub mod weapons;

sdk::export_server!(game::Dust::new());
