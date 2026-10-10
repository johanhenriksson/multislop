pub mod bot;
pub mod client;
pub mod game;
pub mod map;
pub mod shared;

sdk::export_server!(game::Gauntlet::new());
sdk::export_client!(client::RaceClient::new());
