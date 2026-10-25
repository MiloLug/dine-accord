//! Byte-level parity checks for CAR helpers against the Python implementation.

use accord::car;
use std::io::Read;

fn read_section(cursor: &mut std::io::Cursor<&Vec<u8>>) -> Vec<u8> {
    let mut len_buf = [0u8; 4];
    cursor.read_exact(&mut len_buf).expect("read len");
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    cursor.read_exact(&mut buf).expect("read data");
    buf
}

#[test]
fn car_helpers_match_python() {
    let reference_path = "tests/parity_references/car_python_reference.bin";
    if !std::path::Path::new(reference_path).exists() {
        eprintln!("Skipping: {reference_path} not present");
        return;
    }
    let all = std::fs::read(reference_path).expect("read");
    let mut cur = std::io::Cursor::new(&all);

    // CARHEADER and EXTENDED_METADATA are skipped: their version strings follow the Xcode build.
    read_section(&mut cur);
    read_section(&mut cur);
    let py_keyformat = read_section(&mut cur);
    let py_color = read_section(&mut cur);
    let py_packed = read_section(&mut cur);
    let py_svg = read_section(&mut cur);

    assert_eq!(
        car::make_keyformat(&[7, 13, 1, 2, 3, 17, 11, 12]),
        py_keyformat,
        "keyformat"
    );
    assert_eq!(
        car::build_color_csi("AccentColor", 0.5, 0.25, 0.75, 1.0, 1),
        py_color,
        "color"
    );
    assert_eq!(
        car::build_packed_image_csi("img_1", 32, 32, b"BGRA", 4, 6, 0, &car::AtlasKey { scale: 2, ..Default::default() }),
        py_packed,
        "packed"
    );
    assert_eq!(car::build_svg_csi("icon.svg", b"<svg></svg>", 0), py_svg, "svg");
}
