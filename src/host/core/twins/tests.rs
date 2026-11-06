use herdr_agent_kit::agent_procs::fake::FakeProcesses;

use super::*;

const S: &str = "aaaaaaaa-0000-4000-8000-00000000000a";
const T: &str = "bbbbbbbb-0000-4000-8000-00000000000b";

fn table() -> FakeProcesses {
    let mut t = FakeProcesses::default();
    t.insert(40, 1, "claude", "s40", &["claude", "--resume", S]);
    t.insert(50, 1, "sleep", "s50", &["sleep", "9"]);
    t
}

fn runs(pid: Pid, start: &str, session: &str) -> Other {
    Other::Runs(pid, start.into(), session.into())
}

#[test]
fn a_live_process_on_the_same_session_refuses() {
    let got = check(&table(), &[runs(40, "s40", S)], &[S]);
    assert_eq!(got, Err(Refused::Shared));
}

#[test]
fn either_name_of_the_session_counts() {
    let got = check(&table(), &[runs(40, "s40", T)], &[S, T]);
    assert_eq!(got, Err(Refused::Shared));
}

#[test]
fn another_session_refuses_nothing() {
    assert_eq!(check(&table(), &[runs(40, "s40", T)], &[S]), Ok(()));
}

#[test]
fn a_record_of_a_dead_process_or_a_reused_pid_refuses_nothing() {
    assert_eq!(check(&table(), &[runs(99, "s99", S)], &[S]), Ok(()));
    assert_eq!(check(&table(), &[runs(40, "older", S)], &[S]), Ok(()));
}

#[test]
fn a_file_mid_rewrite_refuses_only_under_a_live_claude() {
    let got = check(&table(), &[Other::Unreadable(40)], &[S]);
    assert_eq!(got, Err(Refused::BadRecord));
    assert!(Refused::BadRecord.temporary());
    assert_eq!(
        check(&table(), &[Other::Unreadable(99)], &[S]),
        Ok(()),
        "dead"
    );
    assert_eq!(
        check(&table(), &[Other::Unreadable(50)], &[S]),
        Ok(()),
        "reused"
    );
}

#[test]
fn another_kind_of_record_refuses_nothing() {
    assert_eq!(check(&table(), &[Other::Elsewhere], &[S]), Ok(()));
}
