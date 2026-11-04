//! A herdr server for one scenario, on a Unix socket of its own: each
//! connection is one request line and one reply line (the kit's wire). It
//! answers from the scenario and, like the Python capture's stub, writes a
//! typed prompt into the transcript before it replies: the prompt has landed
//! by the time the delivery looks.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{Value, json};

/// The server's answer to one request: `Ok(result)` or `Err(message)`.
pub type Answer = Result<Value, String>;

/// A running stub: its socket and every request it got (method, params).
pub struct Stub {
    pub socket: PathBuf,
    pub requests: Arc<Mutex<Vec<(String, Value)>>>,
}

impl Stub {
    /// Serves `answer` at `dir/herdr.sock` until the process ends.
    pub fn start(dir: &Path, answer: impl Fn(&str, &Value) -> Answer + Send + 'static) -> Stub {
        let socket = dir.join("herdr.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let mut line = String::new();
                if BufReader::new(&stream).read_line(&mut line).is_err() {
                    continue;
                }
                let Ok(request) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let method = request["method"].as_str().unwrap_or_default().to_string();
                let params = request.get("params").cloned().unwrap_or(Value::Null);
                seen.lock().unwrap().push((method.clone(), params.clone()));
                let reply = match answer(&method, &params) {
                    Ok(result) => json!({"id": request["id"], "result": result}),
                    Err(message) => {
                        json!({"id": request["id"], "error": {"code": "failed", "message": message}})
                    }
                };
                let mut stream = stream;
                let _ = writeln!(stream, "{reply}");
            }
        });
        Stub { socket, requests }
    }

    /// The params of every request of `method`, in order.
    pub fn params_of(&self, method: &str) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == method)
            .map(|(_, p)| p.clone())
            .collect()
    }
}
