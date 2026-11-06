use super::*;

#[test]
fn a_config_directory_comes_only_from_a_transcript_under_projects() {
    let found = config_of("/c/one/projects/-r/a.jsonl");
    assert_eq!(found.as_deref(), Some(Path::new("/c/one")));
    assert_eq!(config_of("/c/one/elsewhere/-r/a.jsonl"), None);
    assert_eq!(config_of("a.jsonl"), None);
}
