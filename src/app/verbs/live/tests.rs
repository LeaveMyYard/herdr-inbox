use std::fs;
use std::path::Path;

use super::CORE_DELIVERY;

fn source(path: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap()
}

/// Whichever of core delivery and the verbs lands second must turn the verbs' side on: left off,
/// a verb says `held`, "check_answers only", while the core worker delivers the answer anyway.
#[test]
fn the_core_switch_is_on_exactly_when_core_delivery_exists() {
    let landed = source("src/store/outcomes/piles.rs").contains("fn undelivered_core");
    assert_eq!(
        CORE_DELIVERY, landed,
        "set CORE_DELIVERY in src/app/verbs/live.rs"
    );
    // the feed's core listing is pinned by behaviour: app::feed::snapshot's tests
    let asked = source("src/app/verbs/live.rs").contains("absent(");
    assert_eq!(asked, landed, "World::core_gone must connect to the socket");
}

/// Any `hc` command starts a core. A verb may only try the socket.
#[test]
fn a_verb_never_runs_hc() {
    for file in ["live.rs", "route.rs", "apply.rs", "show.rs", "mod.rs"] {
        let text = source(&format!("src/app/verbs/{file}"));
        let code: String = text
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect();
        assert!(
            !code.contains("host::core::hc::Hc") && !code.contains("\"hc\""),
            "{file}"
        );
    }
}
