use super::*;

fn text(buf: &Buffer) -> Vec<String> {
    (0..buf.area.height)
        .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
        .collect()
}

#[test]
fn a_toast_shows_for_five_seconds() {
    let mut t = Toasts::default();
    t.push(Toast::new("hi", Severity::Information), 1000);
    assert_eq!(t.next_expiry(), Some(6000));
    t.expire(5999);
    assert_eq!(t.shown().count(), 1);
    t.expire(6000);
    assert_eq!(t.shown().count(), 0);
}

#[test]
fn toasts_stack_at_the_bottom_right_newest_last() {
    let mut t = Toasts::default();
    t.push(Toast::new("older", Severity::Information), 0);
    t.push(Toast::new("newer", Severity::Warning), 0);
    let mut buf = Buffer::empty(Rect::new(0, 0, 40, 20));
    t.draw(&mut buf);
    let rows = text(&buf);
    // 20 wide (half), 3 rows each (padding above and below), a blank line between, one
    // line of margin below
    let edge = format!("{}▌{}", " ".repeat(20), " ".repeat(19));
    assert_eq!(rows[19], " ".repeat(40));
    assert_eq!((&rows[16], &rows[18]), (&edge, &edge));
    assert_eq!(
        rows[17],
        format!("{}▌ newer{}", " ".repeat(20), " ".repeat(13))
    );
    assert_eq!(rows[15], " ".repeat(40), "a line between toasts");
    assert_eq!(
        rows[13],
        format!("{}▌ older{}", " ".repeat(20), " ".repeat(13))
    );
    assert_eq!(buf[(20, 17)].fg, WARNING);
    assert_eq!(buf[(20, 13)].fg, SUCCESS);
}

#[test]
fn long_text_folds_and_a_title_comes_first_in_bold() {
    let mut toast = Toast::new("one two three four five six", Severity::Information);
    toast.title = Some("Title".into());
    assert_eq!(
        lines_of(&toast, 9),
        [
            ("Title".to_string(), true),
            ("one two".to_string(), false),
            ("three".to_string(), false),
            ("four five".to_string(), false),
            ("six".to_string(), false)
        ]
    );
    assert_eq!(fold("abcdefghij", 4), ["abcd", "efgh", "ij"]);
}
