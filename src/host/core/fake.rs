//! A recorded `hc` world for the tests (`tests/fixtures/hc/`): what `hc get`
//! prints, and what was asked.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::Value;

use super::hc::Hc;

/// 0.1.17: `get` of t2, whose foreground is `claude`, pid 20.
pub const GET_17: &str = include_str!("../../../tests/fixtures/hc/get-0.1.17.json");
/// 0.1.18: `get` of t1, a stand-in `claude` under `/bin/sh`, pid 4242.
pub const GET_18: &str = include_str!("../../../tests/fixtures/hc/get-0.1.18.json");

/// `hc`, answering from recordings.
#[derive(Default)]
pub struct FakeHc {
    terminals: BTreeMap<String, Value>,
    /// Every read, in order.
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
        self.terminals.get(address).cloned().ok_or("hc read failed")
    }
}
