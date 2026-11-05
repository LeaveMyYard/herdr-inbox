use std::sync::Mutex;

use super::*;

#[test]
fn queued_marks_run_in_order_off_the_caller_and_drain_waits_for_them() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = Arc::clone(&seen);
    let marks = Marks::start(move |p| {
        std::thread::sleep(Duration::from_millis(20));
        let name = thread::current().name().unwrap_or("").to_string();
        s.lock().unwrap().push((p.map(str::to_string), name));
    });
    marks.push(Some("w1:p1"));
    marks.push(None);
    assert!(marks.drain(Duration::from_secs(5)));
    let seen = seen.lock().unwrap().clone();
    assert_eq!(
        seen,
        [
            (Some("w1:p1".to_string()), "inbox-background".to_string()),
            (None, "inbox-background".to_string())
        ]
    );
}

#[test]
fn drain_is_bounded_and_a_panicking_job_does_not_stop_the_queue() {
    let marks = Marks::start(|p| match p {
        Some("slow") => std::thread::sleep(Duration::from_millis(400)),
        Some("boom") => panic!("a failing job"),
        _ => {}
    });
    marks.push(Some("boom"));
    marks.push(Some("slow"));
    let t = Instant::now();
    assert!(!marks.drain(Duration::from_millis(50)));
    assert!(t.elapsed() < Duration::from_millis(300));
    assert!(marks.drain(Duration::from_secs(5)));
}
