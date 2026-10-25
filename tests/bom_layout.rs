//! BOM layout rules of actool's catalogs, checked on Apple's file and on accord's compile of the
//! same pair. The structural diff can't see any of them.

mod common;

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(b[at..at + 4].try_into().unwrap())
}

/// Every rule violated by the BOM file `b`, as text.
fn layout_violations(b: &[u8]) -> Vec<String> {
    let mut bad = Vec::new();
    let nblocks = be32(b, 12) as usize;
    let index = be32(b, 16) as usize;
    let slots = be32(b, index) as usize;
    let block = |i: usize| (be32(b, index + 4 + 8 * i) as usize, be32(b, index + 8 + 8 * i) as usize);
    let used: Vec<usize> = (1..slots).filter(|&i| block(i) != (0, 0)).collect();

    if block(1).0 != 0x200 {
        bad.push(format!("block 1 at {:#x}", block(1).0));
    }
    if used.last() != Some(&nblocks) {
        bad.push(format!("header counts {nblocks} blocks, highest index {:?}", used.last()));
    }
    let want_slots = 256usize.max((nblocks + 1).next_multiple_of(8));
    if slots != want_slots {
        bad.push(format!("{slots} index slots for {nblocks} blocks, want {want_slots}"));
    }

    let vars = be32(b, 24) as usize;
    let mut at = vars + 4;
    for _ in 0..be32(b, vars) {
        let bi = be32(b, at) as usize;
        let len = b[at + 4] as usize;
        let name = String::from_utf8_lossy(&b[at + 5..at + 5 + len]).into_owned();
        at += 5 + len;
        let (off, _) = block(bi);
        // BITMAPKEYS stores its keys in the entries, so it has no key blocks.
        if &b[off..off + 4] != b"tree" || b[off + 20] != 0 {
            continue;
        }
        let (leaf, _) = block(be32(b, off + 8) as usize);
        for k in 0..usize::from(u16::from_be_bytes([b[leaf + 2], b[leaf + 3]])) {
            let value = be32(b, leaf + 12 + 8 * k);
            let key = be32(b, leaf + 16 + 8 * k);
            if value != key + 1 {
                bad.push(format!("{name} entry {k}: key block {key}, value block {value}"));
                break;
            }
        }
    }
    bad
}

#[test]
fn layout_matches_actool() {
    let root = common::root();
    for name in ["ios/roothide", "daintree"] {
        let s = common::samples().into_iter().find(|s| s.name == name).unwrap();
        let apple_bytes = common::read(&root.join("samples").join(name).join("Assets.car"));
        assert_eq!(layout_violations(&apple_bytes), Vec::<String>::new(), "{name}: Apple's file");

        let apple = accord::car_read::Car::parse(&apple_bytes).unwrap();
        let out = tempfile::TempDir::new().unwrap();
        let ours = std::fs::read(common::compile_pair(&s, &apple, out.path())).unwrap();
        assert_eq!(layout_violations(&ours), Vec::<String>::new(), "{name}: accord's compile");
    }
}
