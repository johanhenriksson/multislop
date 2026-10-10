pub mod bot;
pub mod client;
pub mod game;
pub mod map;
pub mod shared;

sdk::export_server!(game::Arena::new());
sdk::export_client!(client::ArenaClient::new());
