//! The delivery's herdr calls against the kit's FakeHerdr: what each one
//! asks herdr (Python `herdr.py`'s CLI arguments, as socket params), and
//! what a failing herdr reads as (no agent, an unreadable input box, no
//! panel, the error of a prompt).

use herdr_agent_kit::herdr_api::fake::{FakeHerdr, Reply};
use herdr_inbox::claude::Transcripts;
use herdr_inbox::delivery::pane::{composer, live_agent, panel, prompt};
use inbox_core::deliver::Composer;
use serde_json::json;

const S: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const DRAFT: &str = include_str!("fixtures/composer/draft.ansi");

fn screen(text: &str) -> Reply {
    Reply::Result(json!({"read": {"text": text}}))
}

#[test]
fn the_live_agent_is_asked_of_its_pane_and_a_failing_herdr_is_no_agent() {
    let herdr = FakeHerdr::start();
    let dir = tempfile::tempdir().unwrap();
    let files = Transcripts::new(None);
    herdr.on(
        "agent.get",
        Reply::Result(
            json!({"agent": {"agent": "claude", "agent_status": "working", "focused": true,
                                       "agent_session": {"kind": "id", "value": S}}}),
        ),
    );
    let agent = live_agent(&herdr.blocking_client(), &files, dir.path(), "w1:p1").unwrap();
    assert_eq!(
        (agent.kind.as_deref(), agent.status.as_str(), agent.focused),
        (Some("claude"), "working", true)
    );
    assert_eq!(
        (agent.session.as_deref(), agent.reported_session.as_deref()),
        (Some(S), Some(S))
    );
    assert_eq!(
        herdr.params_of("agent.get"),
        vec![json!({"target": "w1:p1"})]
    );
    herdr.on("agent.get", Reply::error("pane_not_found", "no such pane"));
    assert_eq!(
        live_agent(&herdr.blocking_client(), &files, dir.path(), "w1:p1"),
        None
    );
}

#[test]
fn the_input_box_is_read_with_its_colours_and_an_unreadable_one_is_unknown() {
    let herdr = FakeHerdr::start();
    herdr.on("pane.read", screen(DRAFT));
    assert_eq!(composer(&herdr.blocking_client(), "w1:p1"), Composer::Draft);
    let params = herdr.params_of("pane.read");
    assert_eq!(
        (params[0]["lines"].clone(), params[0]["format"].clone()),
        (json!(30), json!("ansi"))
    );
    assert_eq!(params[0]["source"], "visible");
    herdr.on("pane.read", Reply::error("pane_not_found", "gone"));
    assert_eq!(
        composer(&herdr.blocking_client(), "w1:p1"),
        Composer::Unknown
    );
}

#[test]
fn the_panel_is_read_from_the_last_twenty_plain_lines_and_unreadable_is_no_panel() {
    let herdr = FakeHerdr::start();
    let focused = "──────\n❯\n──────\n  ↑/↓ to select · esc";
    herdr.on("pane.read", screen(focused));
    assert!(panel(&herdr.blocking_client(), "w1:p1"));
    let params = herdr.params_of("pane.read");
    assert_eq!(
        (params[0]["lines"].clone(), params[0]["format"].clone()),
        (json!(20), json!("text"))
    );
    herdr.on("pane.read", Reply::error("pane_not_found", "gone"));
    assert!(!panel(&herdr.blocking_client(), "w1:p1"));
}

#[test]
fn a_prompt_is_typed_cleaned_and_its_failure_is_its_error() {
    let herdr = FakeHerdr::start();
    herdr.on("agent.prompt", Reply::Result(json!({})));
    assert_eq!(
        prompt(
            &herdr.blocking_client(),
            "w1:p1",
            "[inbox] Q1 answered\n— yes\x1b[2J"
        ),
        None
    );
    assert_eq!(
        herdr.params_of("agent.prompt"),
        vec![json!({"target": "w1:p1", "text": "[inbox] Q1 answered — yes[2J"})]
    );
    herdr.on("agent.prompt", Reply::error("agent_busy", "fake failure"));
    let error = prompt(&herdr.blocking_client(), "w1:p1", "x").expect("an error");
    assert!(error.contains("fake failure"), "{error}");
}
