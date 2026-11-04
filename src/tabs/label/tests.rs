use std::collections::BTreeMap;

use super::*;

#[test]
fn the_label_shows_the_count_only_when_there_is_one_and_a_mark() {
    let never = || panic!("prefix_unnamed is read only for an unnamed tab with a count");
    assert_eq!(desired_label("web", 2, "❓", never), "❓2 web");
    assert_eq!(desired_label("web", 0, "❓", never), "web");
    assert_eq!(desired_label("web", -1, "❓", never), "web");
    assert_eq!(desired_label("web", 3, "", never), "web");
    assert_eq!(desired_label("6", 0, "❓", never), "6");
}

#[test]
fn an_unnamed_tab_is_prefixed_only_when_asked() {
    // Python: "6".isdigit(), "²".isdigit(), "١٢".isdigit() are True; "½", "" and "6a" are not
    assert_eq!(desired_label("6", 1, "❓", || false), "6");
    assert_eq!(desired_label("6", 1, "❓", || true), "❓1 6");
    assert_eq!(desired_label("²", 1, "!", || false), "²");
    assert_eq!(desired_label("½", 1, "!", || false), "!1 ½");
    assert_eq!(desired_label("", 1, "!", || false), "!1 ");
    assert!(py_isdigit("١٢") && py_isdigit("①") && py_isdigit("６"));
    assert!(!py_isdigit("6a") && !py_isdigit("Ⅷ") && !py_isdigit(""));
}

#[test]
fn counts_add_up_per_tab_and_the_last_listing_of_a_pane_wins() {
    let counts: BTreeMap<String, i64> = [("p1", 2), ("p2", 1), ("p3", 4), ("p4", 7)]
        .into_iter()
        .map(|(p, n)| (p.to_string(), n))
        .collect();
    let tabs = vec![
        ("p1".to_string(), Some("t1".to_string())),
        ("p2".to_string(), Some("t9".to_string())),
        ("p2".to_string(), Some("t1".to_string())),
        ("p3".to_string(), None),
        ("p4".to_string(), Some(String::new())),
    ];
    let got = tab_counts(&counts, &tabs);
    assert_eq!(
        got.into_iter().collect::<Vec<_>>(),
        vec![("t1".to_string(), 3)]
    );
}

#[test]
fn a_tab_may_lose_the_default_mark_and_the_marks_applied_to_it() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE tab_decor (tab_id TEXT, mark TEXT, PRIMARY KEY (tab_id, mark)); \
         INSERT INTO tab_decor VALUES ('t1', '!'), ('t1', ''), ('t2', '#');",
    )
    .unwrap();
    assert_eq!(marks_for(&db, Some("t1")).unwrap(), vec!["!", "❓"]);
    assert_eq!(marks_for(&db, None).unwrap(), vec!["❓"]);
    assert_eq!(marks_for(&db, Some("")).unwrap(), vec!["❓"]);
}
