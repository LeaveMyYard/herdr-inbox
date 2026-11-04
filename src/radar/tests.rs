use super::*;

fn radar_config(text: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("config.toml"), text).unwrap();
    d
}

#[test]
fn radar_carries_the_token_only_with_a_real_extra_tokens_entry() {
    let steps = |text: &str| stepped_aside(Some(radar_config(text).path()));
    assert!(steps("extra_tokens = [\"$inbox_q\"]\n"));
    assert!(steps("[a.b]\nextra_tokens = [{ token = \"inbox_q\" }]\n"));
    assert!(!steps("extra_tokens = []\n# $inbox_q in a comment\n"));
    assert!(!steps("other = [\"$inbox_q\"]\n"));
    assert!(!steps("extra_tokens = [\n"), "a broken file");
    assert!(!stepped_aside(None));
}

#[test]
fn toml_parses_as_tomllib_reads_it() {
    assert!(parses("a = 1\n[b]\nc = \"x\"\n"));
    assert!(!parses("[keys\n"));
    assert!(!parses("a = { b = 1, }\n"), "TOML 1.1's trailing comma");
}

#[test]
fn the_config_is_written_through_a_symlink_with_a_backup() {
    // Python test_radar::test_a_symlinked_config_is_edited_through_the_link
    let d = tempfile::tempdir().unwrap();
    let target = d.path().join("dotfiles.toml");
    std::fs::write(&target, "old\r\n").unwrap();
    let link = d.path().join("config.toml");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(
        !file::write(&link, "changed meanwhile", "new"),
        "only over what was read"
    );
    assert!(file::write(&link, "old\r\n", "new\r\n"));
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new\r\n");
    assert_eq!(
        std::fs::read_to_string(d.path().join("dotfiles.toml.inbox-bak")).unwrap(),
        "old\r\n"
    );
}
