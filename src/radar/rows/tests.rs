use super::*;

const DARK: &str = include_str!("../../../tests/fixtures/radar/dark-font.block.toml");

#[test]
fn every_cell_form_counts_toward_the_row_limit() {
    // Python test_radar::test_every_cell_form_counts_toward_the_16_token_limit
    let mut cells: Vec<String> = vec![
        r#""state_icon""#.into(),
        r#""tab""#.into(),
        r#""$bare""#.into(),
    ];
    cells.extend((0..11).map(|i| format!("{{ token = \"$t{i}\" }}")));
    cells.push(
        r##"{ token = "$title_blocked", fg = "#c04a4a", rules = [{ contains = "[", fg = "#000000" }, { equals = "x" }] }"##
            .into(),
    );
    cells.push(r#"{ token = "$title_unknown" }"#.into());
    let row = format!("[{}]", cells.join(", "));
    assert_eq!(tokens_in(&row), 16);
    assert_eq!(
        edit_row(&row, false),
        row,
        "a 17th cell would break herdr's file"
    );
    assert_eq!(tokens_in("[]"), 0);
    assert_eq!(tokens_in(r#"["$gap"]"#), 1);
}

#[test]
fn our_cell_follows_the_unknown_title_in_the_rows_blocked_red() {
    // Python test_radar::test_matches_the_operators_hand_patch
    let edited = edit_block(DARK, false);
    assert!(edited.contains(
        r##"{ token = "$title_unknown", fg = "#907aa9", bold = false, dim = false }, { token = "$inbox_q", fg = "#c04a4a", bold = true, dim = false }"##
    ));
    assert_eq!(edit_block(&edited, false), edited, "idempotent");
    assert_eq!(edit_block(&edited, true), DARK, "removed byte for byte");
}

#[test]
fn neutral_keeps_every_byte_offset() {
    let text = "a = \"é [x] \\\" #\" # ü [c]\nb = ['[']\n";
    let n = neutral(text);
    assert_eq!(n.len(), text.len());
    assert!(
        !n.contains("[x]") && !n.contains("[c]") && n.contains("b = [' ']"),
        "{n:?}"
    );
}
