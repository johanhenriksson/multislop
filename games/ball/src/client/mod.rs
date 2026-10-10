//! The Ballslop client.
// ponytail: a stub that only predicts; the stadium, players, ball, rockets and HUD come next.

use crate::shared::*;
use sdk::client::*;

#[derive(Default)]
pub struct BallClient;

impl BallClient {
    pub fn new() -> BallClient {
        BallClient
    }
}

impl ClientGame for BallClient {
    fn start(&mut self, _map: &str) -> Vec<Command> {
        Vec::new()
    }

    fn snapshot(&mut self, _snap: &ClientSnapshot) {}

    fn input(&mut self, _raw: &RawInput) -> Vec<u8> {
        encode(&Input::default())
    }

    fn predict(&mut self, me: &mut Entity, input: &[u8], seq: u32, _fresh: bool) {
        predict(me, input, seq);
    }

    fn frame(&mut self, _f: &FrameIn) -> FrameOut {
        FrameOut::default()
    }
}
