//! The layout Apple's actool emits for `--platform iphoneos`: Idiom + Subtype key columns,
//! CoreUI 975 / key-semantics 2, per-idiom keys, `mac`/`ios-marketing` dropped from imagesets.

use accord::compiler;
use std::path::{Path, PathBuf};

fn workspace_tmp(name: &str) -> PathBuf {
    let dir = PathBuf::from("tmp").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_png(path: &Path, w: u32, h: u32, rgba: [u8; 4]) {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba(rgba));
    img.save(path).unwrap();
}

/// iPhone and iPad images, plus `mac` and `ios-marketing` ones that iOS imagesets drop.
fn build_mixed_catalog(root: &Path) {
    let xc = root.join("Images.xcassets");
    let imageset = xc.join("Glyph.imageset");
    std::fs::create_dir_all(&imageset).unwrap();
    std::fs::write(
        xc.join("Contents.json"),
        r#"{"info":{"author":"xcode","version":1}}"#,
    )
    .unwrap();
    write_png(&imageset.join("p1.png"), 10, 10, [255, 0, 0, 255]);
    write_png(&imageset.join("p2.png"), 20, 20, [0, 255, 0, 255]);
    write_png(&imageset.join("p3.png"), 30, 30, [0, 0, 255, 255]);
    std::fs::write(
        imageset.join("Contents.json"),
        r#"{
          "images":[
            {"idiom":"iphone","scale":"1x","filename":"p1.png"},
            {"idiom":"iphone","scale":"2x","filename":"p2.png"},
            {"idiom":"iphone","scale":"3x","filename":"p3.png"},
            {"idiom":"ipad","scale":"1x","filename":"p1.png"},
            {"idiom":"ipad","scale":"2x","filename":"p2.png"},
            {"idiom":"mac","scale":"1x","filename":"p1.png"},
            {"idiom":"ios-marketing","scale":"1x","filename":"p1.png"}
          ],
          "info":{"author":"xcode","version":1}
        }"#,
    )
    .unwrap();
}

fn compile_ios(xcassets: &Path, out: &Path) {
    let plist = out.join("partial.plist");
    compiler::compile(&[(xcassets).to_path_buf()], out, &compiler::Options { platform: ("iphoneos").to_string(), min_deploy: ("14.0").to_string(), app_icon: None, info_plist: (Some(&plist)).map(|p| p.to_path_buf()), accent_color: None, widget_background_color: None, standalone_icon_behavior: ("default").to_string(), target_devices: Vec::new(), include_all_app_icons: false, include_languages: None, development_region: None, plist_localizations: true })
    .expect("compile");
}

fn build_appicon_catalog(root: &Path, entries: &[(&str, &str, &str)]) {
    let xc = root.join("A.xcassets");
    let set = xc.join("AppIcon.appiconset");
    std::fs::create_dir_all(&set).unwrap();
    std::fs::write(
        xc.join("Contents.json"),
        r#"{"info":{"author":"xcode","version":1}}"#,
    )
    .unwrap();
    let mut images = String::from("{\"images\":[");
    for (i, (size, scale, idiom)) in entries.iter().enumerate() {
        let pt: f64 = size.split('x').next().unwrap().parse().unwrap();
        let px = (pt * scale[..1].parse::<f64>().unwrap()).round() as u32;
        let fname = format!("i{px}.png");
        write_png(&set.join(&fname), px, px, [64, 128, 192, 255]);
        if i > 0 {
            images.push(',');
        }
        images.push_str(&format!(
            r#"{{"size":"{size}","idiom":"{idiom}","filename":"{fname}","scale":"{scale}"}}"#
        ));
    }
    images.push_str("],\"info\":{\"author\":\"xcode\",\"version\":1}}");
    std::fs::write(set.join("Contents.json"), images).unwrap();
}

fn compile_ios_icon(xcassets: &Path, out: &Path) {
    let plist = out.join("partial.plist");
    compiler::compile(&[(xcassets).to_path_buf()], out, &compiler::Options { platform: ("iphoneos").to_string(), min_deploy: ("14.0").to_string(), app_icon: (Some("AppIcon")).map(str::to_string), info_plist: (Some(&plist)).map(|p| p.to_path_buf()), accent_color: None, widget_background_color: None, standalone_icon_behavior: ("default").to_string(), target_devices: Vec::new(), include_all_app_icons: false, include_languages: None, development_region: None, plist_localizations: true })
    .expect("compile");
}

fn read_u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
}

/// Parse the KEYFORMAT (`tmfk`) attribute list from a compiled CAR.
fn keyformat(car: &[u8]) -> Vec<u32> {
    let i = car
        .windows(4)
        .position(|w| w == b"tmfk")
        .expect("tmfk block");
    let n = read_u32_le(car, i + 8) as usize;
    (0..n).map(|k| read_u32_le(car, i + 12 + 4 * k)).collect()
}

#[test]
fn ios_imageset_emits_idiom_keyformat_and_header() {
    let root = workspace_tmp("ios_keyformat");
    build_mixed_catalog(&root);
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    compile_ios(&root.join("Images.xcassets"), &out);

    let car = std::fs::read(out.join("Assets.car")).expect("car");

    // Fixed iOS key format: Appearance, Localization, Scale, Idiom, Subtype,
    // Identifier, Element, Part.
    assert_eq!(keyformat(&car), vec![7, 13, 12, 15, 16, 17, 1, 2]);

    let h = car
        .windows(4)
        .position(|w| w == b"RATC")
        .expect("RATC block");
    assert_eq!(read_u32_le(&car, h + 4), 975, "coreui version");
    assert_eq!(read_u32_le(&car, h + 432), 2, "key semantics");

    // EXTENDED_METADATA records the device family, not the SDK name.
    let m = car
        .windows(4)
        .position(|w| w == b"META")
        .expect("META block");
    let platform: String = car[m + 516..m + 516 + 8]
        .iter()
        .take_while(|b| **b != 0)
        .map(|b| *b as char)
        .collect();
    assert_eq!(platform, "ios");
}

#[test]
fn ios_imageset_filters_mac_and_marketing_idioms() {
    let root = workspace_tmp("ios_filter");
    build_mixed_catalog(&root);
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    compile_ios(&root.join("Images.xcassets"), &out);

    let car = std::fs::read(out.join("Assets.car")).expect("car");
    let kf = keyformat(&car);
    let idiom_col = kf.iter().position(|t| *t == 15).expect("idiom column");

    let key_size = kf.len() * 2;
    let mut idioms: Vec<u16> = Vec::new();
    // Heuristic: rendition keys are found by scanning every 2-byte offset, not by walking the tree.
    let scale_col = kf.iter().position(|t| *t == 12).unwrap();
    for i in (0..car.len().saturating_sub(key_size)).step_by(2) {
        let cols: Vec<u16> = (0..kf.len())
            .map(|c| u16::from_le_bytes(car[i + c * 2..i + c * 2 + 2].try_into().unwrap()))
            .collect();
        let scale = cols[scale_col];
        let idiom = cols[idiom_col];
        if (1..=3).contains(&scale) && (1..=2).contains(&idiom) && cols[0] == 0 && cols[1] == 0 {
            idioms.push(idiom);
        }
    }
    assert!(idioms.contains(&1), "expected an iphone (1) rendition");
    assert!(idioms.contains(&2), "expected an ipad (2) rendition");
    assert!(
        !idioms.contains(&6),
        "ios-marketing (6) must be dropped from imagesets"
    );
}

#[test]
fn ios_appicon_emits_loose_primary_pngs() {
    let root = workspace_tmp("ios_appicon_loose");
    build_appicon_catalog(
        &root,
        &[
            ("60x60", "2x", "iphone"),
            ("60x60", "3x", "iphone"),
            ("76x76", "2x", "ipad"),
            ("1024x1024", "1x", "ios-marketing"),
        ],
    );
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    compile_ios_icon(&root.join("A.xcassets"), &out);

    // The marketing (1024) icon is CAR-only.
    assert!(out.join("AppIcon60x60@2x.png").exists(), "iphone loose png");
    assert!(
        out.join("AppIcon76x76@2x~ipad.png").exists(),
        "ipad loose png"
    );
    assert!(!out.join("AppIcon.icns").exists(), "no icns on iOS");
}

#[test]
fn ios_appicon_partial_plist_has_cfbundleicons() {
    let root = workspace_tmp("ios_appicon_plist");
    build_appicon_catalog(
        &root,
        &[("60x60", "2x", "iphone"), ("76x76", "2x", "ipad")],
    );
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    compile_ios_icon(&root.join("A.xcassets"), &out);

    let plist = std::fs::read_to_string(out.join("partial.plist")).unwrap();
    assert!(plist.contains("<key>CFBundleIcons</key>"));
    assert!(plist.contains("<key>CFBundleIcons~ipad</key>"));
    assert!(plist.contains("<key>CFBundlePrimaryIcon</key>"));
    assert!(plist.contains("<string>AppIcon60x60</string>"));
    assert!(plist.contains("<string>AppIcon76x76</string>"));
    assert!(plist.contains("<key>CFBundleIconName</key>"));
    // legacy macOS keys must NOT appear on iOS
    assert!(!plist.contains("<key>CFBundleIconFile</key>"));
}

#[test]
fn ios_appicon_finds_files_named_in_another_case() {
    // actool runs on a case-insensitive file system; RootHide ships `Icon-76@2x.png` for a
    // `Contents.json` entry `icon-76@2x.png`.
    let root = workspace_tmp("ios_appicon_case");
    build_appicon_catalog(&root, &[("60x60", "2x", "iphone"), ("76x76", "2x", "ipad")]);
    let set = root.join("A.xcassets/AppIcon.appiconset");
    std::fs::rename(set.join("i152.png"), set.join("I152.png")).unwrap();
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    compile_ios_icon(&root.join("A.xcassets"), &out);

    assert!(out.join("AppIcon76x76@2x~ipad.png").exists());
}

#[test]
fn ios_appicon_ipad_files_only_with_ipad_icons() {
    // iPhone-only set: CFBundleIcons~ipad carries just the name (no files).
    let root = workspace_tmp("ios_appicon_iphone_only");
    build_appicon_catalog(&root, &[("60x60", "2x", "iphone"), ("60x60", "3x", "iphone")]);
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    compile_ios_icon(&root.join("A.xcassets"), &out);

    let plist = std::fs::read_to_string(out.join("partial.plist")).unwrap();
    assert!(plist.contains("<key>CFBundleIcons~ipad</key>"));
    assert!(!plist.contains("AppIcon76x76"));
    assert!(!out.join("AppIcon76x76@2x~ipad.png").exists());
}

#[test]
fn ios_appicon_renditions_carry_device_idiom() {
    let root = workspace_tmp("ios_appicon_idiom");
    build_appicon_catalog(
        &root,
        &[
            ("60x60", "2x", "iphone"),
            ("60x60", "3x", "iphone"),
            ("1024x1024", "1x", "ios-marketing"),
        ],
    );
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    compile_ios_icon(&root.join("A.xcassets"), &out);

    let car = std::fs::read(out.join("Assets.car")).expect("car");
    let kf = keyformat(&car);
    let idiom_col = kf.iter().position(|t| *t == 15).expect("idiom column");

    let mut seen = std::collections::HashSet::new();
    for i in (0..car.len().saturating_sub(kf.len() * 2)).step_by(2) {
        let cols: Vec<u16> = (0..kf.len())
            .map(|c| u16::from_le_bytes(car[i + c * 2..i + c * 2 + 2].try_into().unwrap()))
            .collect();
        if cols[0] == 0 && cols[1] == 0 && (cols[idiom_col] == 1 || cols[idiom_col] == 6) {
            seen.insert(cols[idiom_col]);
        }
    }
    assert!(seen.contains(&1), "expected a phone (1) icon rendition");
    assert!(seen.contains(&6), "expected a marketing (6) icon rendition");
}

#[test]
fn ios_appicon_keyformat_adds_dim2_and_renditions_dont_collide() {
    let root = workspace_tmp("ios_appicon_keyformat");
    // Sizes sharing idiom and scale differ only in Dimension2.
    build_appicon_catalog(
        &root,
        &[
            ("20x20", "2x", "iphone"),
            ("29x29", "2x", "iphone"),
            ("40x40", "2x", "iphone"),
            ("60x60", "2x", "iphone"),
            ("60x60", "3x", "iphone"),
        ],
    );
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    compile_ios_icon(&root.join("A.xcassets"), &out);

    let car = std::fs::read(out.join("Assets.car")).expect("car");
    let kf = keyformat(&car);

    // Only app icons add Dimension2 (9); see ios_imageset_emits_idiom_keyformat_and_header.
    assert!(kf.contains(&9), "app-icon keyformat must include dim2 (9)");
    let sub = kf.iter().position(|t| *t == 16).unwrap();
    let d2 = kf.iter().position(|t| *t == 9).unwrap();
    assert!(d2 == sub + 1, "dim2 must follow subtype");

    // Dimension2 is the size's rank within the set.
    let d2_col = d2;
    let scale_col = kf.iter().position(|t| *t == 12).unwrap();
    let idiom_col = kf.iter().position(|t| *t == 15).unwrap();
    let mut d2_at_scale2 = std::collections::HashSet::new();
    for i in (0..car.len().saturating_sub(kf.len() * 2)).step_by(2) {
        let cols: Vec<u16> = (0..kf.len())
            .map(|c| u16::from_le_bytes(car[i + c * 2..i + c * 2 + 2].try_into().unwrap()))
            .collect();
        if cols[0] == 0 && cols[1] == 0 && cols[scale_col] == 2 && cols[idiom_col] == 1 {
            d2_at_scale2.insert(cols[d2_col]);
        }
    }
    for idx in [1u16, 2, 3, 4] {
        assert!(
            d2_at_scale2.contains(&idx),
            "expected distinct phone@2x dim2={idx}"
        );
    }
}

#[test]
fn ios_appicon_packs_into_atlases_with_dim1_and_keeps_idiom() {
    let root = workspace_tmp("ios_appicon_packing");
    build_appicon_catalog(
        &root,
        &[
            ("20x20", "2x", "iphone"),
            ("20x20", "3x", "iphone"),
            ("29x29", "2x", "iphone"),
            ("29x29", "3x", "iphone"),
            ("40x40", "2x", "iphone"),
            ("40x40", "3x", "iphone"),
            ("60x60", "2x", "iphone"),
            ("60x60", "3x", "iphone"),
            ("1024x1024", "1x", "ios-marketing"),
        ],
    );
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    compile_ios_icon(&root.join("A.xcassets"), &out);

    let car = std::fs::read(out.join("Assets.car")).expect("car");
    let kf = keyformat(&car);

    // More than one atlas per scale adds Dimension1 (8).
    assert!(kf.contains(&9), "keyformat must carry dim2");
    assert!(kf.contains(&8), "keyformat must carry dim1 once icons pack");

    // The idiom keys the atlas, so packing must keep it.
    let el = kf.iter().position(|t| *t == 1).unwrap();
    let idiom_col = kf.iter().position(|t| *t == 15).unwrap();
    let mut packed_phone = false;
    for i in (0..car.len().saturating_sub(kf.len() * 2)).step_by(2) {
        let cols: Vec<u16> = (0..kf.len())
            .map(|c| u16::from_le_bytes(car[i + c * 2..i + c * 2 + 2].try_into().unwrap()))
            .collect();
        // ELEMENT_PACKED == 9; phone idiom == 1.
        if cols[el] == 9 && cols[idiom_col] == 1 && cols[0] == 0 && cols[1] == 0 {
            packed_phone = true;
        }
    }
    assert!(packed_phone, "expected a phone-idiom packed atlas key");
}

#[test]
fn ios_appicon_multisize_split_per_idiom() {
    let root = workspace_tmp("ios_appicon_msplit");
    build_appicon_catalog(
        &root,
        &[
            ("20x20", "2x", "iphone"),
            ("60x60", "2x", "iphone"),
            ("76x76", "2x", "ipad"),
            ("1024x1024", "1x", "ios-marketing"),
        ],
    );
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    compile_ios_icon(&root.join("A.xcassets"), &out);

    let car = std::fs::read(out.join("Assets.car")).expect("car");
    let kf = keyformat(&car);
    let part_col = kf.iter().position(|t| *t == 2).unwrap();
    let idiom_col = kf.iter().position(|t| *t == 15).unwrap();

    // One MultiSized (part 218) rendition per idiom (phone 1, pad 2, marketing 6).
    let mut multisize_idioms = std::collections::HashSet::new();
    for i in (0..car.len().saturating_sub(kf.len() * 2)).step_by(2) {
        let cols: Vec<u16> = (0..kf.len())
            .map(|c| u16::from_le_bytes(car[i + c * 2..i + c * 2 + 2].try_into().unwrap()))
            .collect();
        if cols[part_col] == 218 && cols[0] == 0 && cols[1] == 0 {
            multisize_idioms.insert(cols[idiom_col]);
        }
    }
    for idiom in [1u16, 2, 6] {
        assert!(
            multisize_idioms.contains(&idiom),
            "expected a MultiSized facet for idiom {idiom}"
        );
    }
}

#[test]
fn ios_appicon_synthesizes_plus_phone_subtype_1792() {
    let root = workspace_tmp("ios_appicon_subtype");
    // A 60pt@3x iPhone icon triggers the synthesized 90pt@2x Plus-phone icon.
    build_appicon_catalog(
        &root,
        &[
            ("60x60", "2x", "iphone"),
            ("60x60", "3x", "iphone"),
            ("1024x1024", "1x", "ios-marketing"),
        ],
    );
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    compile_ios_icon(&root.join("A.xcassets"), &out);

    let car = std::fs::read(out.join("Assets.car")).expect("car");
    let kf = keyformat(&car);
    let sub_col = kf.iter().position(|t| *t == 16).unwrap();
    let part_col = kf.iter().position(|t| *t == 2).unwrap();
    let d2_col = kf.iter().position(|t| *t == 9).unwrap();
    let scale_col = kf.iter().position(|t| *t == 12).unwrap();

    let mut multisize_1792 = false;
    let mut leaf_1792 = false;
    for i in (0..car.len().saturating_sub(kf.len() * 2)).step_by(2) {
        let c: Vec<u16> = (0..kf.len())
            .map(|x| u16::from_le_bytes(car[i + x * 2..i + x * 2 + 2].try_into().unwrap()))
            .collect();
        if c[0] != 0 || c[1] != 0 || c[sub_col] != 1792 {
            continue;
        }
        // Part 218 is the multisize facet, 220 the icon image.
        if c[part_col] == 218 {
            multisize_1792 = true;
        }
        // Dimension2 is the size's rank in the set {60, 90, 1024}.
        if c[part_col] == 220 && c[scale_col] == 2 && c[d2_col] == 2 {
            leaf_1792 = true;
        }
    }
    assert!(multisize_1792, "expected a subtype-1792 multisize facet");
    assert!(leaf_1792, "expected a subtype-1792 90pt leaf icon");
}

#[test]
fn ios_appicon_no_subtype_without_60pt_at_3x() {
    let root = workspace_tmp("ios_appicon_no_subtype");
    build_appicon_catalog(
        &root,
        &[("60x60", "2x", "iphone"), ("1024x1024", "1x", "ios-marketing")],
    );
    let out = root.join("out");
    std::fs::create_dir_all(&out).unwrap();
    compile_ios_icon(&root.join("A.xcassets"), &out);

    let car = std::fs::read(out.join("Assets.car")).expect("car");
    let kf = keyformat(&car);
    let sub_col = kf.iter().position(|t| *t == 16).unwrap();
    let part_col = kf.iter().position(|t| *t == 2).unwrap();
    let mut any_1792 = false;
    for i in (0..car.len().saturating_sub(kf.len() * 2)).step_by(2) {
        let c: Vec<u16> = (0..kf.len())
            .map(|x| u16::from_le_bytes(car[i + x * 2..i + x * 2 + 2].try_into().unwrap()))
            .collect();
        // Constrain to real icon/multisize renditions; the bare value 0x0700
        // (1792) also occurs in unrelated key/metadata bytes.
        if c[0] == 0
            && c[1] == 0
            && c[sub_col] == 1792
            && (c[part_col] == 218 || c[part_col] == 220)
        {
            any_1792 = true;
        }
    }
    assert!(!any_1792, "no subtype-1792 expected without a 60pt@3x icon");
}
