//! A recorded `hc` world for the tests (`tests/fixtures/hc/`): what `hc get`
//! prints, and what was asked.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::Value;

use super::hc::{Hc, NO_TERMINAL, Sent};

/// 0.1.17: `get` of t2, whose foreground is `claude`, pid 20.
pub const GET_17: &str = include_str!("../../../tests/fixtures/hc/get-0.1.17.json");
/// 0.1.18: `get` of t1, a stand-in `claude` under `/bin/sh`, pid 4242.
pub const GET_18: &str = include_str!("../../../tests/fixtures/hc/get-0.1.18.json");

/// `hc`, answering from recordings.
#[derive(Default)]
pub struct FakeHc {
    terminals: BTreeMap<String, Value>,
    screens: BTreeMap<String, String>,
    /// What each send answers, first to last; then [`Sent::Typed`].
    pub answers: Mutex<Vec<Sent>>,
    /// Every read and send, in order.
    pub asked: Mutex<Vec<String>>,
}

impl FakeHc {
    /// A world of the recording `get`.
    pub fn recorded(get: &str) -> FakeHc {
        let mut fake = FakeHc::default();
        fake.set(serde_json::from_str(get).unwrap());
        fake
    }

    /// Adds or replaces the terminal `get` prints for its address.
    pub fn set(&mut self, terminal: Value) {
        let address = terminal["address"].as_str().unwrap().to_owned();
        self.terminals.insert(address, terminal);
    }

    /// The recording of `address`, to change.
    pub fn terminal(&mut self, address: &str) -> &mut Value {
        self.terminals.get_mut(address).unwrap()
    }

    /// What `hc read` prints for `address`.
    pub fn set_screen(&mut self, address: &str, screen: &str) {
        self.screens.insert(address.to_owned(), screen.to_owned());
    }

    /// The sends so far: `address text`.
    pub fn sends(&self) -> Vec<String> {
        let asked = self.asked.lock().unwrap();
        let sends = asked.iter().filter_map(|a| a.strip_prefix("send "));
        sends.map(str::to_owned).collect()
    }

    /// `hc` is not there.
    pub fn gone() -> FakeHc {
        FakeHc::default()
    }

    fn note(&self, what: String) {
        self.asked.lock().unwrap().push(what);
    }
}

impl Hc for FakeHc {
    fn get(&self, address: &str) -> Result<Value, &'static str> {
        self.note(format!("get {address}"));
        self.terminals.get(address).cloned().ok_or(NO_TERMINAL)
    }

    fn screen(&self, address: &str) -> Result<String, &'static str> {
        self.note(format!("read {address}"));
        self.screens.get(address).cloned().ok_or("hc read failed")
    }

    fn send(&self, address: &str, text: &str) -> Sent {
        self.note(format!("send {address} {text}"));
        let mut answers = self.answers.lock().unwrap();
        if answers.is_empty() {
            Sent::Typed
        } else {
            answers.remove(0)
        }
    }
}
