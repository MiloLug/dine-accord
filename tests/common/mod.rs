//! The corpus in tests/corpus, shared by the corpus tests.
// Each test crate uses part of this module.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

#[derive(serde::Deserialize)]
struct Corpus {
    sample: Vec<Sample>,
}

#[derive(serde::Deserialize)]
pub struct Sample {
    pub name: String,
    #[serde(default)]
    pub sources: Vec<String>,
    pub app_icon: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
}

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus")
}

pub fn samples() -> Vec<Sample> {
    let text = std::fs::read_to_string(root().join("corpus.toml")).unwrap();
    toml::from_str::<Corpus>(&text).unwrap().sample
}

/// A corpus file, refusing the Git LFS pointer a clone without `git lfs pull` leaves behind.
pub fn read(path: &Path) -> Vec<u8> {
    let data = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(
        !data.starts_with(b"version https://git-lfs"),
        "{} is a Git LFS pointer: run `git lfs pull`",
        path.display()
    );
    data
}
