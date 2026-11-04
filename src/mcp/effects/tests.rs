use std::sync::Mutex;
use std::time::Duration;

use super::*;

#[derive(Default)]
struct Log(Mutex<Vec<String>>);

impl Herald for Log {
    fn notify(&self, title: &str, body: &str) {
        std::thread::sleep(Duration::from_millis(20));
        self.0
            .lock()
            .unwrap()
            .push(format!("notify {title} / {body}"));
    }
    fn markers(&self, pane: &str) {
        self.0.lock().unwrap().push(format!("markers {pane}"));
    }
}

#[test]
fn effects_run_in_order_off_the_call_and_drain_at_exit() {
    let log = Arc::new(Log::default());
    let fx = Effects::start(log.clone());
    fx.later(vec![
        Effect::Notify {
            title: "Q1 · tab".into(),
            body: "Deploy?".into(),
        },
        Effect::Markers(None),
        Effect::Markers(Some("w1:p1".into())),
    ]);
    assert!(fx.drain(Duration::from_secs(5)));
    assert_eq!(
        *log.0.lock().unwrap(),
        ["notify Q1 · tab / Deploy?", "markers w1:p1"]
    );
    // after the drain nothing is queued any more, and a second drain returns at once
    fx.later(vec![Effect::Markers(Some("x".into()))]);
    assert!(fx.drain(Duration::from_secs(1)));
    assert_eq!(log.0.lock().unwrap().len(), 2);
}

#[test]
fn the_drain_is_bounded() {
    struct Slow;
    impl Herald for Slow {
        fn notify(&self, _: &str, _: &str) {
            std::thread::sleep(Duration::from_secs(2));
        }
        fn markers(&self, _: &str) {}
    }
    let fx = Effects::start(Arc::new(Slow));
    fx.later(vec![Effect::Notify {
        title: "t".into(),
        body: "b".into(),
    }]);
    let at = std::time::Instant::now();
    assert!(!fx.drain(Duration::from_millis(100)));
    assert!(at.elapsed() < Duration::from_secs(1));
}
