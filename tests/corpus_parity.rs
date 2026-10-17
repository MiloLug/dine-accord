//! Compiles each corpus pair and diffs it with Apple's output against
//! `tests/corpus/expectations/<name>.txt`: a new difference fails; `ACCORD_UPDATE_EXPECTATIONS=1`
//! rewrites the files.

mod common;

use accord::car_read::{diff, Car, Payload};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

fn read_car(path: &Path) -> Car {
    Car::parse(&common::read(path)).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()))
}

/// Every file under `dir`, read once so an LFS pointer fails with a hint, not a decode error.
fn check_tree(dir: &Path) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            check_tree(&p);
        } else {
            common::read(&p);
        }
    }
}

#[test]
fn compiled_pairs_match_apple() {
    let root = common::root();
    let update = std::env::var_os("ACCORD_UPDATE_EXPECTATIONS").is_some();
    let mut failures = Vec::new();

    for s in common::samples().iter().filter(|s| s.app_icon.is_some()) {
        let apple = read_car(&root.join("samples").join(&s.name).join("Assets.car"));
        let meta = apple.metadata.as_ref().expect("sample has EXTENDED_METADATA");
        // EXTENDED_METADATA records the device family, actool takes the SDK platform.
        let platform = match meta.deployment_platform.as_str() {
            "ios" => "iphoneos",
            p => p,
        };
        let out = tempfile::TempDir::new().unwrap();
        let inputs: Vec<PathBuf> = s
            .sources
            .iter()
            .map(|u| root.join("sources").join(&s.name).join(u.rsplit('/').next().unwrap()))
            .collect();
        inputs.iter().for_each(|i| check_tree(i));
        let status = Command::new(env!("CARGO_BIN_EXE_accord"))
            .arg("--compile")
            .arg(out.path())
            .args(["--platform", platform])
            .args(["--minimum-deployment-target", &meta.deployment_platform_version])
            .args(["--app-icon", s.app_icon.as_deref().unwrap()])
            .arg("--output-partial-info-plist")
            .arg(out.path().join("partial.plist"))
            .args(&s.args)
            .args(&inputs)
            .output()
            .unwrap();
        assert!(status.status.success(), "{}: {}", s.name, String::from_utf8_lossy(&status.stderr));
        let ours = read_car(&out.path().join("Assets.car"));
        // CoreUI reads rows at TLV 1007's stride: a payload our reader can't decode is broken.
        for r in &ours.renditions {
            if let Payload::Celm { error: Some(e), .. } = &r.payload {
                failures.push(format!("{}: {} undecodable: {e}", s.name, r.name));
            }
        }
        let got: BTreeSet<String> = diff(&apple, &ours).into_iter().collect();

        let file = root.join("expectations").join(format!("{}.txt", s.name.replace('/', "_")));
        let known: BTreeSet<String> = std::fs::read_to_string(&file)
            .map(|t| t.lines().map(str::to_owned).collect())
            .unwrap_or_default();
        if update {
            let text: String = got.iter().map(|l| format!("{l}\n")).collect();
            std::fs::write(&file, text).unwrap();
            continue;
        }
        let gone = known.difference(&got).count();
        if gone > 0 {
            eprintln!("{}: {gone} known differences no longer occur; rerun with ACCORD_UPDATE_EXPECTATIONS=1", s.name);
        }
        let new: Vec<&String> = got.difference(&known).collect();
        if !new.is_empty() {
            failures.push(format!("{}: {} new differences:\n  {}", s.name, new.len(),
                new.iter().take(40).map(|l| l.as_str()).collect::<Vec<_>>().join("\n  ")));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
