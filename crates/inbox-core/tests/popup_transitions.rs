//! The popup model replays the popup transition golden
//! (tests/fixtures/popup-transitions.json, captured from the Python popup by
//! tools/capture_popup_transitions.py): every scenario's events, at their
//! times, give the same state and the same effects, step by step.

mod popup;

use inbox_core::popup::Popup;
use popup::json;
use serde_json::Value;

const GOLDEN: &str = include_str!("../../../tests/fixtures/popup-transitions.json");

/// The keys where two states differ, with both values.
fn differences(want: &Value, got: &Value) -> Vec<String> {
    let (Some(w), Some(g)) = (want.as_object(), got.as_object()) else {
        return vec![format!("want {want}\n  got  {got}")];
    };
    let mut keys: Vec<&String> = w.keys().chain(g.keys()).collect();
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .filter(|k| w.get(*k) != g.get(*k))
        .map(|k| {
            format!(
                "{k}: want {}\n  {:width$} got  {}",
                w.get(k).unwrap_or(&Value::Null),
                "",
                g.get(k).unwrap_or(&Value::Null),
                width = k.len()
            )
        })
        .collect()
}

fn replay(scenario: &Value) -> Result<(), String> {
    let name = scenario["name"].as_str().unwrap();
    let steps = scenario["steps"].as_array().unwrap();
    let first = &steps[0];
    let (mut model, effects) = Popup::open(
        json::open(&first["event"]["open"]),
        first["at"].as_u64().unwrap(),
    );
    let mut results = vec![(effects, json::state(&model))];
    for step in &steps[1..] {
        let effects = model.handle(json::event(&step["event"]), step["at"].as_u64().unwrap());
        results.push((effects, json::state(&model)));
    }
    for (i, (step, (effects, state))) in steps.iter().zip(results).enumerate() {
        let effects = Value::Array(effects.iter().map(json::effect).collect());
        let event = match step["event"].get("key") {
            Some(k) => format!("key {k}"),
            None => step["event"]
                .as_object()
                .unwrap()
                .keys()
                .next()
                .unwrap()
                .clone(),
        };
        let at = &step["at"];
        if effects != step["effects"] {
            return Err(format!(
                "{name} step {i} ({event} at {at}): effects\n  want {}\n  got  {effects}",
                step["effects"]
            ));
        }
        if state != step["state"] {
            let diff = differences(&step["state"], &state).join("\n  ");
            return Err(format!(
                "{name} step {i} ({event} at {at}): state\n  {diff}"
            ));
        }
    }
    Ok(())
}

fn golden() -> Value {
    serde_json::from_str(GOLDEN).unwrap()
}

/// Replay the golden's scenario `name`.
fn scenario(name: &str) {
    let golden = golden();
    let scenario = golden["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == name);
    let scenario = scenario.unwrap_or_else(|| panic!("no scenario {name} in the golden"));
    if let Err(diff) = replay(scenario) {
        panic!("{diff}");
    }
}

/// One test per scenario, named after the Python test it replays
/// (`<test file without test_>::<test without test_>`); the ledger maps the
/// Python tests to these.
macro_rules! scenarios {
    ($($module:ident { $($test:ident = $name:literal,)* })*) => {
        $(mod $module { $( #[test] fn $test() { super::scenario($name) } )* })*
        const LISTED: &[&str] = &[$($($name,)*)*];
    };
}

scenarios! {
    tui {
        group_list_rows_and_order = "tui.group_list_rows_and_order",
        single_question_layout_and_keys = "tui.single_question_layout_and_keys",
        single_pick_sends_immediately_and_closes = "tui.single_pick_sends_immediately_and_closes",
        type_something_and_multi_select = "tui.type_something_and_multi_select",
        multi_select_submit_needs_a_pick = "tui.multi_select_submit_needs_a_pick",
        chat_about_this_and_escape = "tui.chat_about_this_and_escape",
        chat_about_this_and_escape_esc = "tui.chat_about_this_and_escape.esc",
        empty_state_closes_on_any_key = "tui.empty_state_closes_on_any_key",
        answering_drafts_and_moves_to_the_next_tab = "tui.answering_drafts_and_moves_to_the_next_tab",
        submit_is_per_group_and_keeps_blanks = "tui.submit_is_per_group_and_keeps_blanks",
        cancel_and_drafts_survive_a_restart = "tui.cancel_and_drafts_survive_a_restart",
        cancel_and_drafts_survive_a_restart_again = "tui.cancel_and_drafts_survive_a_restart.again",
        new_groups_never_join_an_open_group = "tui.new_groups_never_join_an_open_group",
        one_becomes_several_while_open = "tui.one_becomes_several_while_open",
        dismiss_question_in_group_and_whole_group_in_list = "tui.dismiss_question_in_group_and_whole_group_in_list",
        tab_order_focused_pane_first_groups_adjacent = "tui.tab_order_focused_pane_first_groups_adjacent",
    }
    background_keys {
        arrival_into_the_empty_popup_does_not_take_over = "background_keys.arrival_into_the_empty_popup_does_not_take_over",
        arrival_then_deliberate_tab_enables_immediate_send = "background_keys.arrival_then_deliberate_tab_enables_immediate_send",
        typing_after_the_question_closes_never_reaches_app_keys = "background_keys.typing_after_the_question_closes_never_reaches_app_keys",
        keys_are_swallowed_briefly_after_a_background_view_change = "background_keys.keys_are_swallowed_briefly_after_a_background_view_change",
        gone_tab_disables_g_and_d = "background_keys.gone_tab_disables_g_and_d",
        dismiss_needs_y = "background_keys.dismiss_needs_y",
    }
    cursor_targets {
        cancel_stays_cancel_when_a_question_arrives = "cursor_targets.cancel_stays_cancel_when_a_question_arrives",
        item_cursor_follows_its_question = "cursor_targets.item_cursor_follows_its_question",
        vanished_target_parks_on_nothing = "cursor_targets.vanished_target_parks_on_nothing",
    }
    group_list {
        opens_straight_into_the_only_group = "group_list.opens_straight_into_the_only_group",
        opens_straight_into_the_only_group_not_with_a_warning_entry = "group_list.opens_straight_into_the_only_group.not_with_a_warning_entry",
        closed_row_is_inert_until_the_operator_moves = "group_list.closed_row_is_inert_until_the_operator_moves",
        cursor_target_vanishing_from_a_return_parks_inert = "group_list.cursor_target_vanishing_from_a_return_parks_inert",
        g_from_the_list_goes_to_the_groups_agent = "group_list.g_from_the_list_goes_to_the_groups_agent",
        last_group_sent_closes_the_popup = "group_list.last_group_sent_closes_the_popup",
    }
    arrivals {
        tab_enter_after_arrival_never_answers_the_new_question = "arrivals.tab_enter_after_arrival_never_answers_the_new_question",
        arrivals_are_list_rows_and_never_move_the_cursor = "arrivals.arrivals_are_list_rows_and_never_move_the_cursor",
        arrival_into_the_empty_view_still_opens_with_tab = "arrivals.arrival_into_the_empty_view_still_opens_with_tab",
        question_view_is_armed_only_after_a_moment = "arrivals.question_view_is_armed_only_after_a_moment",
        list_and_lone_question_are_armed_too = "arrivals.list_and_lone_question_are_armed_too",
    }
    race {
        withdraw_mid_typing_never_answers_the_other_agent = "race.withdraw_mid_typing_never_answers_the_other_agent",
        cursor_on_an_option_when_the_question_closes = "race.cursor_on_an_option_when_the_question_closes",
        several_to_one_does_not_make_enter_send = "race.several_to_one_does_not_make_enter_send",
        new_question_mid_typing_keeps_the_text = "race.new_question_mid_typing_keeps_the_text",
        go_to_agent_works_for_multi_select = "race.go_to_agent_works_for_multi_select",
        failed_delivery_entry_retry_and_hide = "race.failed_delivery_entry_retry_and_hide",
    }
    real_timing {
        list_wrap_with_real_arming = "real_timing.list_wrap_with_real_arming",
        list_row_that_just_arrived_is_not_selectable = "real_timing.list_row_that_just_arrived_is_not_selectable",
        question_view_ignores_keys_until_armed = "real_timing.question_view_ignores_keys_until_armed",
        submit_tab_ignores_keys_until_armed = "real_timing.submit_tab_ignores_keys_until_armed",
        undelivered_view_ignores_keys_until_armed = "real_timing.undelivered_view_ignores_keys_until_armed",
    }
    alerts {
        popup_shows_alerts_first_unnumbered_and_d_acknowledges = "alerts.popup_shows_alerts_first_unnumbered_and_d_acknowledges",
        popup_d_acknowledges_an_alert_and_g_goes_to_the_pane = "alerts.popup_d_acknowledges_an_alert_and_g_goes_to_the_pane",
        popup_g_goes_to_the_alerts_pane = "alerts.popup_g_goes_to_the_alerts_pane",
        an_alert_withdrawn_while_on_screen_becomes_inert = "alerts.an_alert_withdrawn_while_on_screen_becomes_inert",
        m1_enter_enter_never_goes_to_a_blocked_agent = "alerts.m1_enter_enter_never_goes_to_a_blocked_agent",
        m1_a_blocked_alert_view_goes_only_on_a_deliberate_g = "alerts.m1_a_blocked_alert_view_goes_only_on_a_deliberate_g",
        h1_habitual_sequences_with_an_alert_open = "alerts.h1_habitual_sequences_with_an_alert_open",
        h1_habitual_1_g_with_an_alert_open = "alerts.h1_habitual_1_g_with_an_alert_open",
        h1_the_only_question_still_opens_directly_with_an_alert_open = "alerts.h1_the_only_question_still_opens_directly_with_an_alert_open",
        a_held_g_never_passes_the_pause = "alerts.a_held_g_never_passes_the_pause",
        a_double_tapped_g_never_passes_the_pause = "alerts.a_double_tapped_g_never_passes_the_pause",
        a_deliberate_g_after_a_pause_still_goes = "alerts.a_deliberate_g_after_a_pause_still_goes",
        m1_enter_enter_never_acknowledges_an_agent_alert = "alerts.m1_enter_enter_never_acknowledges_an_agent_alert",
    }
    failed_pile {
        q167_gone_answers_collapse_into_one_row_and_d_hides_them_all = "failed_pile.q167_gone_answers_collapse_into_one_row_and_d_hides_them_all",
        q167_a_single_gone_answer_keeps_its_own_row = "failed_pile.q167_a_single_gone_answer_keeps_its_own_row",
        q167_a_gone_answer_arriving_later_never_joins_the_open_summary = "failed_pile.q167_a_gone_answer_arriving_later_never_joins_the_open_summary",
    }
    agent_gone {
        pending_rows_say_they_are_already_answered = "agent_gone.pending_rows_say_they_are_already_answered",
        popup_does_not_list_hidden_rows = "agent_gone.popup_does_not_list_hidden_rows",
    }
    deep_link {
        popup_opens_on_the_requested_question = "deep_link.popup_opens_on_the_requested_question",
        popup_opens_on_the_group_for_each_target_kind = "deep_link.popup_opens_on_the_group_for_each_target_kind",
        gone_target_falls_back_to_the_list_with_a_notice = "deep_link.gone_target_falls_back_to_the_list_with_a_notice",
        a_deep_linked_view_waits_for_a_deliberate_key = "deep_link.a_deep_linked_view_waits_for_a_deliberate_key",
        no_hold_opens_like_a_normal_popup = "deep_link.no_hold_opens_like_a_normal_popup",
        no_request_keeps_the_normal_start = "deep_link.no_request_keeps_the_normal_start",
    }
    edge {
        note_box_keys = "edge.note_box_keys",
        confirm_under_tab = "edge.confirm_under_tab",
        empty_view_keys = "edge.empty_view_keys",
        empty_view_up_closes = "edge.empty_view_up_closes",
        review_keys = "edge.review_keys",
        list_digits_skip_alerts = "edge.list_digits_skip_alerts",
        closed_group_poll_inside_list = "edge.closed_group_poll_inside_list",
        tab_wraps_and_switch_in_list = "edge.tab_wraps_and_switch_in_list",
        tab_in_the_list = "edge.tab_in_the_list",
        tab_in_a_failed_view = "edge.tab_in_a_failed_view",
        tab_in_an_alert_view = "edge.tab_in_an_alert_view",
        tab_in_the_empty_view = "edge.tab_in_the_empty_view",
        held_deep_link_takes_no_key_until_tab = "edge.held_deep_link_takes_no_key_until_tab",
        poll_under_a_confirmation = "edge.poll_under_a_confirmation",
        gone_summary_row_keys = "edge.gone_summary_row_keys",
        sent_nothing = "edge.sent_nothing",
        sent_part = "edge.sent_part",
        focus_failed = "edge.focus_failed",
        failed_entry_ends_on_screen = "edge.failed_entry_ends_on_screen",
        question_closed_in_a_lone_group = "edge.question_closed_in_a_lone_group",
        multi_select_arming_and_digits = "edge.multi_select_arming_and_digits",
        reopen_a_note_and_the_typed_text = "edge.reopen_a_note_and_the_typed_text",
        selected_text_in_a_reopened_box = "edge.selected_text_in_a_reopened_box",
        tab_in_a_box_lone = "edge.tab_in_a_box_lone",
        tab_in_a_box_group = "edge.tab_in_a_box_group",
        tab_in_a_box_then_d = "edge.tab_in_a_box_then_d",
        shift_tab_in_a_box = "edge.shift_tab_in_a_box",
        tab_twice_in_the_list = "edge.tab_twice_in_the_list",
        box_editing_keys = "edge.box_editing_keys",
        tab_in_a_box_with_a_preview = "edge.tab_in_a_box_with_a_preview",
        tab_in_a_box_with_a_preview_lone = "edge.tab_in_a_box_with_a_preview_lone",
        failed_view_enter_arms = "edge.failed_view_enter_arms",
        alert_view_d_arms = "edge.alert_view_d_arms",
        reopen_after_a_part_submit = "edge.reopen_after_a_part_submit",
        submit_reads_the_drafts_it_showed = "edge.submit_reads_the_drafts_it_showed",
        empty_escape = "edge.empty_escape",
        empty_escape_with_arrivals = "edge.empty_escape_with_arrivals",
        open_after_an_unseen_close = "edge.open_after_an_unseen_close",
        open_after_an_unseen_close_of_all = "edge.open_after_an_unseen_close_of_all",
        list_d_after_an_unseen_close = "edge.list_d_after_an_unseen_close",
        confirm_after_an_unseen_close = "edge.confirm_after_an_unseen_close",
        draft_after_an_unseen_close = "edge.draft_after_an_unseen_close",
        submit_tab_digits_arm = "edge.submit_tab_digits_arm",
        g_right_after_the_popup_opens = "edge.g_right_after_the_popup_opens",
    }
    remote_sync {
        popup_lists_and_answers_a_remote_question = "remote_sync.popup_lists_and_answers_a_remote_question",
        popup_dismisses_a_remote_group_with_confirmation = "remote_sync.popup_dismisses_a_remote_group_with_confirmation",
        popup_shows_an_offline_machine_and_a_rejection = "remote_sync.popup_shows_an_offline_machine_and_a_rejection",
        deep_link_to_a_remote_question = "remote_sync.deep_link_to_a_remote_question",
        deep_link_to_a_remote_question_held = "remote_sync.deep_link_to_a_remote_question.held",
        a_remote_question_closed_there_becomes_an_inert_placeholder = "remote_sync.a_remote_question_closed_there_becomes_an_inert_placeholder",
        m1_the_popup_shows_a_past_expiry_as_expired = "remote_sync.m1_the_popup_shows_a_past_expiry_as_expired",
        m2_the_popup_lists_it_and_d_clears_it = "remote_sync.m2_the_popup_lists_it_and_d_clears_it",
        popup_acknowledges_a_remote_alert = "remote_sync.popup_acknowledges_a_remote_alert",
        m1_an_answer_to_the_question_on_screen_never_lands_on_a_replacement = "remote_sync.m1_an_answer_to_the_question_on_screen_never_lands_on_a_replacement",
    }
    remote_edge {
        only_remote_rows = "remote_edge.only_remote_rows",
        answering_one_of_two = "remote_edge.answering_one_of_two",
        last_remote_answer_closes = "remote_edge.last_remote_answer_closes",
        enter_and_g_on_remote_rows = "remote_edge.enter_and_g_on_remote_rows",
        remote_view_keys = "remote_edge.remote_view_keys",
        chat_about_this = "remote_edge.chat_about_this",
        multi_select_and_empty_text = "remote_edge.multi_select_and_empty_text",
        arrivals_are_armed = "remote_edge.arrivals_are_armed",
        remote_rows_end = "remote_edge.remote_rows_end",
        remote_alert_ends_on_screen = "remote_edge.remote_alert_ends_on_screen",
        remote_alert_d_in_the_list = "remote_edge.remote_alert_d_in_the_list",
        queued_row_is_not_cleared = "remote_edge.queued_row_is_not_cleared",
        dismiss_kept_and_polled_under = "remote_edge.dismiss_kept_and_polled_under",
        unlabelled_machine = "remote_edge.unlabelled_machine",
        remote_question_ends_after_answer_elsewhere = "remote_edge.remote_question_ends_after_answer_elsewhere",
    }
}

#[test]
fn every_scenario_of_the_golden_has_its_test() {
    let golden = golden();
    let mut names: Vec<&str> = golden["scenarios"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    let mut listed = LISTED.to_vec();
    names.sort_unstable();
    listed.sort_unstable();
    assert_eq!(golden["schema"], 1);
    assert_eq!(
        names, listed,
        "the golden was regenerated: list its scenarios here"
    );
}

#[test]
fn the_delays_are_the_pythons() {
    use inbox_core::popup::{GO_PAUSE, GRACE, RELEASE, ROW_ARM, VIEW_ARM};
    let golden = golden();
    let d = &golden["delays_ms"];
    assert_eq!(
        ["arm", "row_arm", "grace", "release", "go"].map(|k| d[k].as_u64().unwrap()),
        [VIEW_ARM, ROW_ARM, GRACE, RELEASE, GO_PAUSE]
    );
}
