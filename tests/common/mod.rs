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

/// Compiles a pair's sources the way actool built Apple's catalog: the platform, deployment
/// target and Xcode build stored in it, the sample's `--app-icon` and `args`. Returns the
/// accord catalog's path inside `out`.
pub fn compile_pair(s: &Sample, apple: &accord::car_read::Car, out: &Path) -> PathBuf {
    let meta = apple.metadata.as_ref().expect("sample has EXTENDED_METADATA");
    // EXTENDED_METADATA records the device family, actool takes the SDK platform.
    let platform = match meta.deployment_platform.as_str() {
        "ios" => "iphoneos",
        p => p,
    };
    // CoreUI 1006 is the first Xcode 27 build.
    let coreui = apple.header.as_ref().expect("sample has CARHEADER").coreui_version;
    let xcode = if coreui >= 1006 { "27" } else { "26" };
    let inputs: Vec<PathBuf> = s
        .sources
        .iter()
        .map(|u| root().join("sources").join(&s.name).join(u.rsplit('/').next().unwrap()))
        .collect();
    inputs.iter().for_each(|i| check_tree(i));
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_accord"))
        .arg("--compile")
        .arg(out)
        .args(["--platform", platform])
        .args(["--minimum-deployment-target", &meta.deployment_platform_version])
        .args(["--app-icon", s.app_icon.as_deref().expect("compile pair")])
        .args(["--xcode-version", xcode])
        .arg("--output-partial-info-plist")
        .arg(out.join("partial.plist"))
        .args(&s.args)
        .args(&inputs)
        .output()
        .unwrap();
    assert!(status.status.success(), "{}: {}", s.name, String::from_utf8_lossy(&status.stderr));
    out.join("Assets.car")
}

/// Every file under `dir`, read once so an LFS pointer fails with a hint, not a decode error.
fn check_tree(dir: &Path) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            check_tree(&p);
        } else {
            read(&p);
        }
    }
}
