//! Compiles the reference catalog (tests/ref_samples/Catalog.xcassets) and checks the outputs.

use accord::{bom::BomWriter, car, catalog::AssetCatalog, compiler};
use std::path::PathBuf;

fn ref_catalog() -> PathBuf {
    PathBuf::from("tests/ref_samples/Catalog.xcassets")
}

fn workspace_tmp(name: &str) -> PathBuf {
    let dir = PathBuf::from("tmp").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn compile_reference_catalog_produces_outputs() {
    if !ref_catalog().exists() {
        eprintln!("Skipping: reference catalog missing");
        return;
    }
    let out = workspace_tmp("rust_e2e_full");
    let plist = out.join("AppIcon.Info.plist");
    let files = compiler::compile(&[(&ref_catalog()).to_path_buf()], &out, &compiler::Options { platform: ("macosx").to_string(), min_deploy: ("11.0").to_string(), app_icon: (Some("AppIcon")).map(str::to_string), info_plist: (Some(&plist)).map(|p| p.to_path_buf()), accent_color: None, widget_background_color: None, standalone_icon_behavior: ("default").to_string(), target_devices: Vec::new(), include_all_app_icons: false, include_languages: None, development_region: None, plist_localizations: true, ..Default::default() })
    .expect("compile");
    assert!(files.iter().any(|p| p.ends_with("Assets.car")));
    assert!(files.iter().any(|p| p.ends_with("AppIcon.icns")));
    assert!(files.iter().any(|p| p.ends_with("AppIcon.Info.plist")));
    let car_bytes = std::fs::read(out.join("Assets.car")).expect("read car");
    assert_eq!(&car_bytes[..8], b"BOMStore");
    let plist_txt = std::fs::read_to_string(&plist).expect("read plist");
    assert!(plist_txt.contains("<string>AppIcon</string>"));
}

#[test]
fn compile_reference_catalog_without_icon() {
    if !ref_catalog().exists() {
        return;
    }
    let out = workspace_tmp("rust_e2e_no_icon");
    let files = compiler::compile(&[(&ref_catalog()).to_path_buf()], &out, &compiler::Options { platform: ("macosx").to_string(), min_deploy: ("11.0").to_string(), app_icon: None, info_plist: None, accent_color: None, widget_background_color: None, standalone_icon_behavior: ("default").to_string(), target_devices: Vec::new(), include_all_app_icons: false, include_languages: None, development_region: None, plist_localizations: true, ..Default::default() })
    .expect("compile");
    assert!(files.iter().any(|p| p.ends_with("Assets.car")));
    assert!(!files.iter().any(|p| p.ends_with(".icns")));
    assert!(!files.iter().any(|p| p.ends_with(".plist")));
}

#[test]
fn version_matches_expected_fields() {
    // Compile-time check that the library still exposes what the CLI uses.
    let hdr = car::make_carheader_full(0, accord::xcode::XcodeVersion::default(), "macosx", 1);
    assert_eq!(&hdr[..4], b"RATC");
    let _ = BomWriter::new();
}

#[test]
fn catalog_parse_does_not_crash() {
    if !ref_catalog().exists() {
        return;
    }
    let mut catalog = AssetCatalog::new(
        ref_catalog(),
        "macosx".to_string(),
        "11.0".to_string(),
        Some("AppIcon".to_string()),
        None,
        None,
    );
    let (renditions, facets) = catalog.parse().expect("parse");
    assert!(!renditions.is_empty(), "expected renditions");
    assert!(!facets.is_empty(), "expected facets");
    assert!(facets.contains_key("AppIcon"));
}
