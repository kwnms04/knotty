//! The generated header is the source of truth, so it must be reproducible
//! from the Rust source and it must be usable by a C consumer.
//!
//! Run with `KNOTTY_UPDATE_HEADER=1` to rewrite the committed header.

use std::path::{Path, PathBuf};
use std::process::Command;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn include_dir() -> PathBuf {
    crate_dir().join("../../include")
}

fn generate() -> String {
    let dir = crate_dir();
    let config =
        cbindgen::Config::from_file(dir.join("cbindgen.toml")).expect("read cbindgen.toml");
    let mut generated = Vec::new();
    cbindgen::Builder::new()
        .with_crate(&dir)
        .with_config(config)
        .generate()
        .expect("generate header")
        .write(&mut generated);
    String::from_utf8(generated).expect("header is UTF-8")
}

#[test]
fn committed_header_matches_what_cbindgen_generates() {
    let path = include_dir().join("knotty.h");
    let generated = generate();

    if std::env::var_os("KNOTTY_UPDATE_HEADER").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).expect("create include dir");
        std::fs::write(&path, &generated).expect("write header");
        return;
    }

    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        committed,
        generated,
        "{} is stale — regenerate with KNOTTY_UPDATE_HEADER=1",
        path.display(),
    );
}

#[test]
fn a_c_consumer_compiles_against_the_committed_header() {
    let consumer = crate_dir().join("tests/consumer.c");
    let compiler = std::env::var("CC").unwrap_or_else(|_| "cc".to_owned());

    let output = Command::new(&compiler)
        .args(["-std=c11", "-Wall", "-Werror", "-fsyntax-only"])
        .arg("-I")
        .arg(include_dir())
        .arg(&consumer)
        .output()
        .unwrap_or_else(|error| panic!("run {compiler}: {error}"));

    assert!(
        output.status.success(),
        "{}:\n{}",
        Path::new(&consumer).display(),
        String::from_utf8_lossy(&output.stderr),
    );
}
