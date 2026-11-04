use super::*;

#[test]
fn only_one_ordered_block_is_edited() {
    let block = format!("{START}\n[x]\n{END}\n");
    assert!(block_span(&block).is_some());
    assert!(
        block_span(&format!("{block}{block}")).is_none(),
        "two blocks"
    );
    assert!(
        block_span(&format!("{END}\n{START}\n")).is_none(),
        "out of order"
    );
    assert!(block_span("no block").is_none());
}

#[test]
fn tables_of_arrays_and_multiline_strings_are_refused() {
    let wrap = |inner: &str| format!("a = 1\n{START}\n{inner}{END}\n");
    assert!(unsupported(&wrap(
        "[[ui.sidebar.agents.extra]]\nname = \"x\"\n"
    )));
    assert!(unsupported(&wrap("note = \"\"\"multi\nline\"\"\"\n")));
    assert!(unsupported(&wrap("note = '''x'''\n")));
    assert!(!unsupported(&wrap("[ui.sidebar.agents]\nrows = []\n")));
    assert!(
        !unsupported(&format!("[[outside]]\n{}", wrap(""))),
        "outside the block"
    );
}

#[test]
fn an_edit_that_would_not_parse_is_dropped_with_a_warning() {
    let text = format!(
        "{START}\n[ui.sidebar.agents]\nrows = [[{{ token = \"$title_unknown\" }}]]\n{END}\n"
    );
    let mut warned = Vec::new();
    let got = edited(
        &text,
        false,
        &|t| Ok(!t.contains("$inbox_q")),
        Path::new("/c"),
        &mut |m| warned.push(m.to_string()),
    );
    assert_eq!(got.unwrap(), None);
    assert_eq!(
        warned,
        vec!["radar sidebar: edit would break /c; left alone"]
    );
    assert!(
        edited(&text, false, &|_| Ok(true), Path::new("/c"), &mut |_| {})
            .unwrap()
            .is_some()
    );
}
