pub mod bot;
pub mod client;
pub mod game;
pub mod map;
pub mod shared;
pub mod weapons;

sdk::export_server!(game::Dust::new());
sdk::export_client!(client::DustClient::new());
