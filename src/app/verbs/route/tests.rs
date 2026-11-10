use super::{Effects, Mark, Route, Send, deliver};

#[derive(Default)]
struct Log {
    gone: bool,
    no_worker: bool,
    no_sender: bool,
    calls: Vec<String>,
}

impl Effects for Log {
    fn ensure_worker(&mut self, server: Option<&str>) -> bool {
        self.calls.push(format!("worker {server:?}"));
        !self.no_worker
    }
    fn server_gone(&mut self, server: Option<&str>) -> bool {
        self.calls.push(format!("ask {server:?}"));
        self.gone
    }
    fn core_gone(&mut self) -> bool {
        self.calls.push("ask core".to_string());
        self.gone
    }
    fn start_sender(&mut self, questions: &[i64], force: bool, socket: Option<&str>) -> bool {
        self.calls
            .push(format!("send {questions:?} {force} {socket:?}"));
        !self.no_sender
    }
    fn refresh_marks(&mut self, _: &[Mark]) {}
}

fn tui(server: Option<&str>, session: bool) -> Route {
    Route::Tui {
        server: server.map(str::to_string),
        session,
    }
}

fn send(route: Route) -> Vec<Send> {
    vec![Send {
        route,
        questions: vec![1, 2],
    }]
}

fn run(route: Route, retry: bool, core: bool, gone: bool) -> (Option<&'static str>, Vec<String>) {
    let mut log = Log {
        gone,
        ..Log::default()
    };
    (deliver(&send(route), retry, core, &mut log), log.calls)
}

#[test]
fn the_worker_comes_first_then_the_question_then_the_sender() {
    let (delivery, calls) = run(tui(Some("/s"), true), false, false, false);
    assert_eq!(delivery, Some("queued"));
    assert_eq!(
        calls,
        [
            r#"worker Some("/s")"#,
            r#"ask Some("/s")"#,
            r#"send [1, 2] false Some("/s")"#
        ]
    );
}

#[test]
fn a_server_proven_gone_keeps_the_worker_and_loses_the_sender() {
    let (delivery, calls) = run(tui(Some("/s"), true), true, false, true);
    assert_eq!(delivery, Some("held"));
    assert_eq!(calls, [r#"worker Some("/s")"#, r#"ask Some("/s")"#]);
}

#[test]
fn retry_forces_a_pane_and_the_default_socket_needs_a_session() {
    let (delivery, calls) = run(tui(None, true), true, false, false);
    assert_eq!(delivery, Some("queued"));
    assert_eq!(calls, ["worker None", "ask None", "send [1, 2] true None"]);
}

/// The recipient rule beats retry: with no session a forced send could reach another agent.
#[test]
fn no_server_and_no_session_is_held_even_for_retry_and_asks_nothing() {
    let (delivery, calls) = run(tui(None, false), true, false, false);
    assert_eq!(delivery, Some("held"));
    assert_eq!(calls, ["worker None"]);
}

#[test]
fn nowhere_is_held_and_keeps_its_worker() {
    let (delivery, calls) = run(Route::Nowhere { server: None }, false, false, false);
    assert_eq!(delivery, Some("held"));
    assert_eq!(calls, ["worker None"]);
}

#[test]
fn before_core_delivery_a_core_answer_starts_nothing_at_all() {
    let (delivery, calls) = run(Route::Core, true, false, false);
    assert_eq!(delivery, Some("held"));
    assert!(calls.is_empty(), "{calls:?}");
}

#[test]
fn from_core_delivery_a_core_answer_is_never_forced_and_sets_no_socket() {
    let (delivery, calls) = run(Route::Core, true, true, false);
    assert_eq!(delivery, Some("queued"));
    assert_eq!(
        calls,
        [
            r#"worker Some("herdr-core")"#,
            "ask core",
            "send [1, 2] false None"
        ]
    );
    let (delivery, calls) = run(Route::Core, false, true, true);
    assert_eq!(delivery, Some("held"));
    assert_eq!(calls, [r#"worker Some("herdr-core")"#, "ask core"]);
}

#[test]
fn nothing_to_deliver_starts_nothing() {
    let mut log = Log::default();
    assert_eq!(deliver(&[], true, true, &mut log), None);
    assert!(log.calls.is_empty());
}

/// `queued` for a batch is a claim about every answer in it.
#[test]
fn one_answer_with_no_sender_makes_the_whole_result_held_and_every_worker_goes_first() {
    let mut log = Log::default();
    let both = [
        Send {
            route: tui(Some("/a"), true),
            questions: vec![1],
        },
        Send {
            route: Route::Nowhere {
                server: Some("/b".to_string()),
            },
            questions: vec![2],
        },
    ];
    assert_eq!(deliver(&both, false, false, &mut log), Some("held"));
    assert_eq!(
        log.calls,
        [
            r#"worker Some("/a")"#,
            r#"worker Some("/b")"#,
            r#"ask Some("/a")"#,
            r#"send [1] false Some("/a")"#
        ]
    );
}

fn with(no_worker: bool, no_sender: bool, route: Route) -> Option<&'static str> {
    let mut log = Log {
        no_worker,
        no_sender,
        ..Log::default()
    };
    deliver(&send(route), false, true, &mut log)
}

/// `queued` is a claim that something will carry the answer: never true with neither.
#[test]
fn queued_needs_a_sender_that_started_or_a_worker_that_runs() {
    let pane = || tui(Some("/s"), true);
    assert_eq!(with(false, false, pane()), Some("queued"));
    assert_eq!(
        with(true, false, pane()),
        Some("queued"),
        "the sender started"
    );
    assert_eq!(
        with(false, true, pane()),
        Some("queued"),
        "the worker has it"
    );
    assert_eq!(with(true, true, pane()), Some("held"), "nobody has it");
    assert_eq!(with(true, true, Route::Core), Some("held"));
    assert_eq!(with(false, true, Route::Core), Some("queued"));
}
