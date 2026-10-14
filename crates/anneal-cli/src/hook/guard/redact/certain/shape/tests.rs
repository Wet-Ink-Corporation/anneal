//! Each slot shape, by the words it takes and refuses.

use super::Shape;

/// Each shape, words it accepts, and words it refuses.
const CASES: &[(Shape, &[&str], &[&str])] = &[
    (Shape::Any, &["x", "-x"], &[""]),
    (
        Shape::Digits,
        &["0", "10", "007", "999999999"],
        &["", "x10", "1.5", "-1", "+1", "1 ", "1234567890"],
    ),
    (
        Shape::Count,
        &["1", "10", "01", "999999999"],
        &["", "0", "00", "-1", "1234567890"],
    ),
    (
        Shape::Integer,
        &["5", "-5", "+30", "-999999999"],
        &["", "-", "+-5", "5x", "--5", "99999999999999999999"],
    ),
    (
        Shape::Number,
        &["1", "0.5", ".5", "5.", "999999999.999999999"],
        &[
            "",
            ".",
            "1.2.3",
            "x",
            "-1",
            "1e3",
            "1234567890.5",
            "1.1234567890",
        ],
    ),
    (
        Shape::Duration,
        &["5", "1.5m", "2h", ".5d", "10s"],
        &["", "s", "5x", "5ms", "-5", "1234567890s"],
    ),
    (
        Shape::Mask,
        &["0xff", "0XFF", "3", "deadbeef", "0x10"],
        &["", "0x", "0xfg", "0-3", "-1", "0", "0x0", "000"],
    ),
    (
        Shape::CpuList,
        &["0", "0-3", "0-3,5,8-15:2"],
        &["", "ff", ",1", "-1", "0 1"],
    ),
    (
        Shape::Size,
        &["0", "4096", "4K", "4KB", "4KiB", "1M", "2k", "999999999G"],
        &[
            "",
            "L",
            "x",
            "K",
            "4X",
            "4KiBs",
            "4iB",
            "-1",
            "l",
            "1T",
            "1Z",
            "1234567890",
        ],
    ),
    (
        Shape::Buffering,
        &["L", "0", "4KiB"],
        &["", "l", "1Z", "LL"],
    ),
    (Shape::Variable, &["HOME", "x", "1"], &["", "a=b", "="]),
    (
        Shape::Blocks,
        &["100", "+100M", "-2k", "1c"],
        &["", "M", "100X", "1.5M", "1234567890k"],
    ),
    (
        Shape::Mode,
        &["644", "-644", "/222", "u+w", "u=rwx,g=rx", "-u+s", "+w"],
        &["", "-", "8", "rwx", "u+q", "644x", "+644", "-w", "/w"],
    ),
    (
        Shape::Words("auto never"),
        &["auto", "never"],
        &["", "always", "aut", "auto never"],
    ),
];

#[test]
fn each_shape_takes_its_syntax_and_refuses_the_rest() {
    let mut wrong = Vec::new();
    for &(shape, accepted, refused) in CASES {
        for text in accepted.iter().filter(|text| !shape.accepts(text)) {
            wrong.push(format!("{shape:?} refuses {text:?}"));
        }
        for text in refused.iter().filter(|text| shape.accepts(text)) {
            wrong.push(format!("{shape:?} accepts {text:?}"));
        }
    }

    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
