//! The `engine` file has three readers: `scripts/launch`, Rust's and Python's
//! `runs_rust`. One table drives all three, so they cannot disagree on a
//! spelling; only the exact first line `rust` is Rust.

use std::fs;
use std::path::Path;
use std::process::Command;

use herdr_inbox::worker::spawn::runs_rust;

use crate::world::{Root, repo, stand_in, stdout};

const SHAPES: [(&[u8], bool); 20] = [
    (b"rust\n", true),
    (b"rust", true),
    (b"rust\nand a second line\n", true),
    (b"rust\n\xff\xfe\n", true),
    (b"rust\n\0", true),
    (b"rust\r\n", false),
    (b"rust\r", false),
    (b"ru\0st\n", false),
    (b"rust\0\n", false),
    (b"rust\0", false),
    (b"\0rust\n", false),
    (b"rus", false),
    (b"python\n", false),
    (b"\n", false),
    (b"", false),
    (b"\nrust\n", false),
    (b" rust\n", false),
    (b"rust \n", false),
    (b"RUST\n", false),
    (b"\xef\xbb\xbfrust\n", false),
];

fn python_says(root: &Path) -> bool {
    let code = "import sys; sys.path.insert(0, sys.argv[1]); from herdr_inbox import worker; \
                print(worker.runs_rust(sys.argv[2]))";
    let out = Command::new("python3")
        .args(["-I", "-c", code])
        .arg(repo().join("src"))
        .arg(root)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    stdout(&out).trim() == "True"
}

fn launcher_says(r: &Root) -> bool {
    !stdout(&r.sh("launch", &["which"], "", &[])).is_empty()
}

fn all_three(r: &Root, want: bool, what: &str) {
    assert_eq!(launcher_says(r), want, "scripts/launch: {what}");
    assert_eq!(runs_rust(&r.root()), want, "Rust: {what}");
    assert_eq!(python_says(&r.root()), want, "Python: {what}");
}

#[test]
fn every_reader_agrees_on_every_shape() {
    let r = Root::new();
    stand_in(&r.root().join("bin/herdr-inbox"), "own", "1.0.0", 2, 0);
    for (bytes, want) in SHAPES {
        fs::write(r.root().join("engine"), bytes).unwrap();
        all_three(&r, want, &String::from_utf8_lossy(bytes));
    }
}

#[test]
fn no_readable_file_is_python_everywhere_and_silent() {
    let r = Root::new();
    stand_in(&r.root().join("bin/herdr-inbox"), "own", "1.0.0", 2, 0);
    let file = r.root().join("engine");
    fs::remove_file(&file).unwrap();
    all_three(&r, false, "no file");
    fs::create_dir(&file).unwrap();
    all_three(&r, false, "a directory");
    assert!(r.sh("launch", &["which"], "", &[]).stderr.is_empty());
    fs::remove_dir(&file).unwrap();
    std::os::unix::fs::symlink(r.root().join("nowhere"), &file).unwrap();
    all_three(&r, false, "a dangling symlink");
}
