use std::time::Duration;

use super::*;

#[test]
fn another_servers_answers_are_left_to_its_worker() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(5)).unwrap();
    store
        .db()
        .execute_batch(
            "INSERT INTO questions (id, created_at, question, norm, status) VALUES \
             (1, 'x', 'a', 'a', 'answered'), (2, 'x', 'b', 'b', 'answered'), (3, 'x', 'c', 'c', 'answered'), \
             (4, 'x', 'd', 'd', 'answered'); \
             INSERT INTO question_ext (id, group_id, herdr_server) VALUES (2, 2, '/srv/b'), (3, 3, ''), \
             (4, 4, '/srv/a');",
        )
        .unwrap();
    let qs: Vec<_> = (1..=4).map(|i| store.get(i).unwrap().unwrap()).collect();
    let mut asked = Vec::new();
    let kept = local_answers(&store, &qs, false, "/srv/a", &mut |s| {
        asked.push(s.map(str::to_string))
    })
    .unwrap();
    assert_eq!(kept.iter().map(|q| q.id).collect::<Vec<_>>(), vec![1, 4]);
    // an empty server is another's in Python (`not in (None, mine)`)
    assert_eq!(
        asked,
        vec![None, Some("/srv/b".to_string()), Some(String::new())]
    );
    asked.clear();
    let kept = local_answers(&store, &qs[..1], true, "/srv/a", &mut |s| {
        asked.push(s.map(str::to_string))
    })
    .unwrap();
    assert_eq!(kept.len(), 1);
    assert!(
        asked.is_empty(),
        "a retry is the worker's own: nothing to start"
    );
}

#[test]
fn a_core_answer_is_this_processs_to_deliver_and_its_worker_is_made_sure_of() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Store::open_migrating(&dir.path().join("inbox.db"), Duration::from_secs(5)).unwrap();
    store
        .db()
        .execute_batch(
            "INSERT INTO questions (id, created_at, question, norm, status) VALUES \
             (1, 'x', 'a', 'a', 'answered'), (2, 'x', 'b', 'b', 'answered'); \
             INSERT INTO question_ext (id, group_id, herdr_server) VALUES \
             (1, 1, 'herdr-core'), (2, 2, 'herdr-core');",
        )
        .unwrap();
    let qs: Vec<_> = (1..=2).map(|i| store.get(i).unwrap().unwrap()).collect();
    let mut asked = Vec::new();
    let kept = local_answers(&store, &qs, true, "/srv/a", &mut |s| {
        asked.push(s.map(str::to_string))
    })
    .unwrap();
    assert_eq!(kept.iter().map(|q| q.id).collect::<Vec<_>>(), vec![1, 2]);
    assert_eq!(
        asked,
        vec![Some("herdr-core".to_string())],
        "one worker, once"
    );
}
