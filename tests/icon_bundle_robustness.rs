//! icon.json shapes Apple accepts give the car/plist/icns set; inputs Apple rejects give `Err`
//! with a useful message, never a panic or an empty success.

use accord::icon_bundle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static TMP_SEQ: AtomicU32 = AtomicU32::new(0);

fn tmpdir() -> PathBuf {
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "actool_robust_{}_{seq}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_synthetic_png(path: &Path, size: u32, rgba: [u8; 4]) {
    let buf =
        image::ImageBuffer::from_pixel(size, size, image::Rgba(rgba));
    image::DynamicImage::ImageRgba8(buf).save(path).unwrap();
}

fn make_bundle(stem: &str, icon_json: &str) -> (PathBuf, PathBuf) {
    let parent = tmpdir();
    let bundle = parent.join(format!("{stem}.icon"));
    std::fs::create_dir_all(bundle.join("Assets")).unwrap();
    write_synthetic_png(&bundle.join("Assets/main.png"), 1024, [80, 160, 200, 255]);
    std::fs::write(bundle.join("icon.json"), icon_json).unwrap();
    let out = parent.join("out");
    std::fs::create_dir_all(&out).unwrap();
    (bundle, out)
}

fn compile_ok(stem: &str, icon_json: &str) -> Vec<PathBuf> {
    let (bundle, out) = make_bundle(stem, icon_json);
    let plist = out.join("info.plist");
    accord::compiler::compile(&[(&bundle).to_path_buf()], &out, &accord::compiler::Options { platform: ("macosx").to_string(), min_deploy: ("26.0").to_string(), app_icon: (Some(stem)).map(str::to_string), info_plist: (Some(&plist)).map(|p| p.to_path_buf()), accent_color: None, standalone_icon_behavior: ("default").to_string(), ..Default::default() })
    .expect("compile should succeed")
}

fn compile_err(stem: &str, icon_json: &str) -> String {
    let (bundle, out) = make_bundle(stem, icon_json);
    let plist = out.join("info.plist");
    let res = accord::compiler::compile(&[(&bundle).to_path_buf()], &out, &accord::compiler::Options { platform: ("macosx").to_string(), min_deploy: ("26.0").to_string(), app_icon: (Some(stem)).map(str::to_string), info_plist: (Some(&plist)).map(|p| p.to_path_buf()), accent_color: None, standalone_icon_behavior: ("default").to_string(), ..Default::default() });
    match res {
        Ok(files) => panic!("expected Err, got Ok({files:?})"),
        Err(e) => format!("{e}"),
    }
}

fn assert_three_file_output(files: &[PathBuf], stem: &str) {
    assert!(
        files.iter().any(|p| p.ends_with("Assets.car")),
        "missing Assets.car in {files:?}"
    );
    assert!(
        files.iter().any(|p| p.ends_with("info.plist")),
        "missing info.plist in {files:?}"
    );
    assert!(
        files.iter().any(|p| p.ends_with(&format!("{stem}.icns"))),
        "missing {stem}.icns in {files:?}"
    );
}

#[test]
fn multi_group_bundle_compiles() {
    let files = compile_ok(
        "MultiG",
        r#"{
          "groups": [
            {"name":"A","layers":[{"image-name":"main.png","name":"a"}]},
            {"name":"B","layers":[{"image-name":"main.png","name":"b"}]}
          ]
        }"#,
    );
    assert_three_file_output(&files, "MultiG");
}

#[test]
fn anonymous_groups_get_sequenced_fallback_names() {
    let files = compile_ok(
        "Anon",
        r#"{
          "groups": [
            {"layers":[{"image-name":"main.png","name":"a"}]},
            {"layers":[{"image-name":"main.png","name":"b"}]},
            {"layers":[{"image-name":"main.png","name":"c"}]}
          ]
        }"#,
    );
    assert_three_file_output(&files, "Anon");
}

#[test]
fn hidden_layer_does_not_break_compilation() {
    let files = compile_ok(
        "Hidden",
        r#"{
          "fill":"automatic",
          "groups":[{"name":"G","layers":[{"image-name":"main.png","name":"a","hidden":true}]}]
        }"#,
    );
    assert_three_file_output(&files, "Hidden");
}

#[test]
fn glass_layer_does_not_break_compilation() {
    let files = compile_ok(
        "Glass",
        r#"{
          "fill":"automatic",
          "groups":[{"name":"G","layers":[{"image-name":"main.png","name":"a","glass":true}]}]
        }"#,
    );
    assert_three_file_output(&files, "Glass");
}

#[test]
fn linear_gradient_top_level_fill_compiles() {
    let files = compile_ok(
        "Grad",
        r#"{
          "fill":{"linear-gradient":["srgb:1,0,0,1","srgb:0,0,1,1"]},
          "groups":[{"name":"G","layers":[{"image-name":"main.png","name":"a"}]}]
        }"#,
    );
    assert_three_file_output(&files, "Grad");
}

#[test]
fn grayscale_source_with_variant_axis_compiles() {
    // A grayscale source loads as GA8, but the fill-specializations path converts sized
    // renditions to GA8/GA16 from BGRA: read as BGRA, it overran a slice (feishin.icon).
    let parent = tmpdir();
    let bundle = parent.join("Gray.icon");
    std::fs::create_dir_all(bundle.join("Assets")).unwrap();
    write_synthetic_png(&bundle.join("Assets/main.png"), 1024, [128, 128, 128, 255]);
    std::fs::write(
        bundle.join("icon.json"),
        r#"{
          "fill-specializations":[
            {"value":{"linear-gradient":["display-p3:1,1,1,1","display-p3:0.5,0.5,0.5,1"]}},
            {"appearance":"dark","value":"system-dark"}
          ],
          "groups":[{"name":"G","layers":[{"image-name":"main.png","name":"a"}]}]
        }"#,
    )
    .unwrap();
    let out = parent.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let plist = out.join("info.plist");
    let files = accord::compiler::compile(&[(&bundle).to_path_buf()], &out, &accord::compiler::Options { platform: ("macosx").to_string(), min_deploy: ("26.0").to_string(), app_icon: (Some("Gray")).map(str::to_string), info_plist: (Some(&plist)).map(|p| p.to_path_buf()), accent_color: None, standalone_icon_behavior: ("default").to_string(), ..Default::default() })
    .expect("grayscale bundle with a dark fill should compile, not panic");
    assert_three_file_output(&files, "Gray");
}

#[test]
fn automatic_gradient_fill_compiles() {
    let files = compile_ok(
        "AutoGrad",
        r#"{
          "fill":{"automatic-gradient":"srgb:1,0,0,1"},
          "groups":[{"name":"G","layers":[{"image-name":"main.png","name":"a"}]}]
        }"#,
    );
    assert_three_file_output(&files, "AutoGrad");
}

#[test]
fn supported_platforms_shared_keyword_compiles() {
    let files = compile_ok(
        "Shared",
        r#"{
          "groups":[{"name":"G","layers":[{"image-name":"main.png","name":"a"}]}],
          "supported-platforms":{"squares":"shared"}
        }"#,
    );
    assert_three_file_output(&files, "Shared");
}

#[test]
fn missing_icon_json_returns_clean_error() {
    let parent = tmpdir();
    let bundle = parent.join("NoJson.icon");
    std::fs::create_dir_all(bundle.join("Assets")).unwrap();
    write_synthetic_png(&bundle.join("Assets/main.png"), 64, [0, 0, 0, 255]);
    let out = parent.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let plist = out.join("info.plist");
    let res = accord::compiler::compile(&[(&bundle).to_path_buf()], &out, &accord::compiler::Options { platform: ("macosx").to_string(), min_deploy: ("26.0").to_string(), app_icon: (Some("NoJson")).map(str::to_string), info_plist: (Some(&plist)).map(|p| p.to_path_buf()), accent_color: None, standalone_icon_behavior: ("default").to_string(), ..Default::default() });
    let msg = res.expect_err("must err").to_string();
    assert!(msg.contains("icon.json"), "msg should mention icon.json: {msg}");
}

#[test]
fn bundle_is_file_returns_clean_error() {
    let parent = tmpdir();
    let bundle = parent.join("FileBundle.icon");
    std::fs::write(&bundle, b"not a directory").unwrap();
    let out = parent.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let plist = out.join("info.plist");
    let res = accord::compiler::compile(&[(&bundle).to_path_buf()], &out, &accord::compiler::Options { platform: ("macosx").to_string(), min_deploy: ("26.0").to_string(), app_icon: (Some("FileBundle")).map(str::to_string), info_plist: (Some(&plist)).map(|p| p.to_path_buf()), accent_color: None, standalone_icon_behavior: ("default").to_string(), ..Default::default() });
    assert!(res.is_err(), "bundle that is a file must error");
}

#[test]
fn empty_object_icon_json_returns_clean_error() {
    // Apple errors on `{}` (no `groups`), although serde's default would accept it.
    let msg = compile_err("EmptyObj", "{}");
    assert!(
        msg.contains("groups"),
        "error should mention missing `groups`: {msg}"
    );
}

#[test]
fn empty_groups_array_succeeds_with_no_outputs() {
    // Apple accepts `{"groups": []}`.
    let (bundle, out) = make_bundle("EmptyGroups", r#"{"groups":[]}"#);
    let plist = out.join("info.plist");
    let files = accord::compiler::compile(&[(&bundle).to_path_buf()], &out, &accord::compiler::Options { platform: ("macosx").to_string(), min_deploy: ("26.0").to_string(), app_icon: (Some("EmptyGroups")).map(str::to_string), info_plist: (Some(&plist)).map(|p| p.to_path_buf()), accent_color: None, standalone_icon_behavior: ("default").to_string(), ..Default::default() })
    .expect("empty groups array must not error");
    assert_eq!(files, vec![std::fs::canonicalize(&plist).unwrap()], "no source images → only the plist");
}

#[test]
fn layer_without_image_name_returns_clean_error() {
    let msg = compile_err(
        "NoImg",
        r#"{"groups":[{"layers":[{"name":"only_name"}]}]}"#,
    );
    assert!(
        msg.contains("does not have an image name"),
        "msg should describe the missing image-name: {msg}"
    );
    assert!(msg.contains("only_name"), "msg should include layer name: {msg}");
}

#[test]
fn missing_referenced_image_returns_clean_error() {
    let msg = compile_err(
        "BadRef",
        r#"{"groups":[{"layers":[{"image-name":"nope.png","name":"x"}]}]}"#,
    );
    assert!(
        msg.contains("does not exist"),
        "msg should say image does not exist: {msg}"
    );
    assert!(msg.contains("nope.png"), "msg should name the missing file: {msg}");
}

#[test]
fn malformed_json_returns_clean_error() {
    let parent = tmpdir();
    let bundle = parent.join("Mal.icon");
    std::fs::create_dir_all(bundle.join("Assets")).unwrap();
    write_synthetic_png(&bundle.join("Assets/main.png"), 64, [0, 0, 0, 255]);
    std::fs::write(bundle.join("icon.json"), "{ \"groups\": [").unwrap();
    let out = parent.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let plist = out.join("info.plist");
    let res = accord::compiler::compile(&[(&bundle).to_path_buf()], &out, &accord::compiler::Options { platform: ("macosx").to_string(), min_deploy: ("26.0").to_string(), app_icon: (Some("Mal")).map(str::to_string), info_plist: (Some(&plist)).map(|p| p.to_path_buf()), accent_color: None, standalone_icon_behavior: ("default").to_string(), ..Default::default() });
    assert!(res.is_err(), "malformed JSON must error");
}

#[test]
fn nonstring_image_name_returns_clean_error() {
    let msg = compile_err(
        "NSImg",
        r#"{"groups":[{"layers":[{"image-name":42,"name":"x"}]}]}"#,
    );
    // Either serde or the image-name validator may reject it.
    assert!(!msg.is_empty());
}

#[test]
fn zero_byte_image_returns_clean_error() {
    let parent = tmpdir();
    let bundle = parent.join("Z.icon");
    std::fs::create_dir_all(bundle.join("Assets")).unwrap();
    std::fs::write(bundle.join("Assets/main.png"), b"").unwrap();
    std::fs::write(
        bundle.join("icon.json"),
        r#"{"groups":[{"layers":[{"image-name":"main.png","name":"x"}]}]}"#,
    )
    .unwrap();
    let out = parent.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let plist = out.join("info.plist");
    let res = accord::compiler::compile(&[(&bundle).to_path_buf()], &out, &accord::compiler::Options { platform: ("macosx").to_string(), min_deploy: ("26.0").to_string(), app_icon: (Some("Z")).map(str::to_string), info_plist: (Some(&plist)).map(|p| p.to_path_buf()), accent_color: None, standalone_icon_behavior: ("default").to_string(), ..Default::default() });
    assert!(res.is_err(), "zero-byte PNG must error");
}

#[test]
fn fifty_groups_compile_without_panic() {
    // Apple rejects this with "Icon validation failed"; accepting it is fine, a panic is not.
    let mut groups = String::from("[");
    for i in 0..50 {
        if i > 0 {
            groups.push(',');
        }
        groups.push_str(&format!(
            r#"{{"name":"G{i}","layers":[{{"image-name":"main.png","name":"L{i}"}}]}}"#
        ));
    }
    groups.push(']');
    let files = compile_ok(
        "Many",
        &format!(r#"{{"groups":{groups}}}"#),
    );
    assert_three_file_output(&files, "Many");
}

#[test]
fn is_icon_bundle_recognizes_extension_alone() {
    // A broken `.icon` must get the `.icon` compiler's error, not fall through to xcassets.
    let parent = tmpdir();
    let dir = parent.join("Just.icon");
    std::fs::create_dir_all(&dir).unwrap();
    assert!(icon_bundle::is_icon_bundle(&dir));

    let file = parent.join("File.icon");
    std::fs::write(&file, b"").unwrap();
    assert!(icon_bundle::is_icon_bundle(&file));

    let other = parent.join("Plain.xcassets");
    std::fs::create_dir_all(&other).unwrap();
    assert!(!icon_bundle::is_icon_bundle(&other));
}
