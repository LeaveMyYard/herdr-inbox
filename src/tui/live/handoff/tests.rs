use std::time::{Duration, Instant};

use super::*;

#[test]
fn the_detached_sender_runs_deliver_with_the_ids_and_force() {
    // test_spawn_delivery_runs_inbox_deliver_detached: a stand-in binary records its args
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("args");
    let exe = dir.path().join("stand-in");
    std::fs::write(
        &exe,
        format!(
            "#!/bin/sh\necho \"$@\" > {}.tmp && mv {0}.tmp {0}\n",
            out.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    let h = LiveHandoff {
        herdr: Herdr::new(herdr_agent_kit::herdr_api::ServerId::new(
            dir.path().join("no-herdr.sock"),
        )),
        state_dir: dir.path().to_path_buf(),
        server: String::new(),
        exe: Some(exe),
    };
    h.spawn_delivery(&[7, 8], true);
    let t = Instant::now();
    while !out.exists() && t.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        "deliver 7 8 --force\n"
    );
    assert_eq!(deliver_args(&[3], false), ["deliver", "3"]);
}

#[test]
fn a_sender_that_cannot_start_is_only_logged() {
    let dir = tempfile::tempdir().unwrap();
    let h = LiveHandoff {
        herdr: Herdr::new(herdr_agent_kit::herdr_api::ServerId::new(
            dir.path().join("no-herdr.sock"),
        )),
        state_dir: dir.path().to_path_buf(),
        server: String::new(),
        exe: Some(dir.path().join("missing-binary")),
    };
    h.spawn_delivery(&[1], false); // must not panic
}
