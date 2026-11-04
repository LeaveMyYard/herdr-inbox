use std::path::PathBuf;

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
    assert!(parses("a = 1\n[b]\nc = \"x\"\n").unwrap());
    assert!(!parses("[keys\n").unwrap());
    assert!(
        !parses("a = { b = 1, }\n").unwrap(),
        "TOML 1.1's trailing comma"
    );
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

/// herdr with radar installed; `meanwhile` runs when radar's config dir is
/// asked (between the read and the write), and the file is recorded at the
/// reload.
struct Host<'a> {
    meanwhile: Box<dyn FnMut() + 'a>,
    path: PathBuf,
    reloaded: Vec<String>,
}

impl crate::surface::Surface for Host<'_> {
    fn set_open_count(&mut self, _: &str, _: i64) {}
    fn pane_tabs(&mut self) -> Option<Vec<(String, Option<String>)>> {
        None
    }
    fn all_tabs(&mut self) -> Option<Vec<(String, Option<String>)>> {
        None
    }
    fn tab_label(&mut self, _: &str) -> Option<String> {
        None
    }
    fn rename_tab(&mut self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn notify(&mut self, _: &str, _: &str) {}
    fn open_popup(&mut self, _: &[(String, String)]) -> Result<(), String> {
        Ok(())
    }
    fn radar_installed(&mut self) -> bool {
        true
    }
    fn radar_config_dir(&mut self) -> Option<PathBuf> {
        (self.meanwhile)();
        None
    }
    fn reload_config(&mut self) {
        self.reloaded
            .push(std::fs::read_to_string(&self.path).unwrap());
    }
}

const BLOCK: &str = include_str!("../../tests/fixtures/radar/dark-font.block.toml");

#[test]
fn a_write_meanwhile_wins_and_herdr_reloads_only_what_was_written() {
    // Python test_radar::test_a_concurrent_write_wins
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("config.toml");
    let original = format!("[keys]\n{BLOCK}");
    std::fs::write(&path, &original).unwrap();
    let rewritten = format!("{original}# radar rewrote its block\n");
    let mut host = Host {
        meanwhile: Box::new(|| std::fs::write(&path, &rewritten).unwrap()),
        path: path.clone(),
        reloaded: Vec::new(),
    };
    assert_eq!(
        ensure(&path, false, &mut host, &mut |_| {}),
        Status::Conflict
    );
    assert!(host.reloaded.is_empty(), "no reload after a refused write");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), rewritten);
    assert!(!d.path().join("config.toml.inbox-bak").exists());
    let mut host = Host {
        meanwhile: Box::new(|| {}),
        path: path.clone(),
        reloaded: Vec::new(),
    };
    assert_eq!(
        ensure(&path, false, &mut host, &mut |_| {}),
        Status::Changed
    );
    assert_eq!(host.reloaded.len(), 1);
    assert!(
        host.reloaded[0].contains("$inbox_q"),
        "herdr reloads the edited file"
    );
}

#[test]
fn a_write_after_the_temporary_file_is_made_is_refused_too() {
    // the re-check right before the rename (Python `_write`'s second read)
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("config.toml");
    std::fs::write(&path, "a\n").unwrap();
    let probe = file::write_checked(&path, "a\n", "b\n", &mut || {
        std::fs::write(&path, "c\n").unwrap()
    });
    assert!(!probe);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "c\n");
    let leftovers: Vec<_> = std::fs::read_dir(d.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(
        leftovers.is_empty(),
        "the temporary file is removed: {leftovers:?}"
    );
}
