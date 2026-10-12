use std::ffi::OsStr;

use super::{Level, Window, WindowSource, crossing, reading, window};

#[test]
fn the_env_window_wins_and_is_capped_at_the_model_window() {
    let from_env = window(Some(OsStr::new("500000")), Some(1_000_000));
    assert_eq!(
        from_env,
        Window {
            tokens: 500_000,
            source: WindowSource::Env
        },
        "the variable sets the auto-compact window"
    );
    let capped = window(Some(OsStr::new("500000")), Some(200_000));
    assert_eq!(
        capped.tokens, 200_000,
        "never past the model's own context window"
    );
}

#[test]
fn the_env_window_is_read_as_claude_code_reads_it() {
    assert_eq!(
        window(Some(OsStr::new("500k")), None).tokens,
        100_000,
        "`500k` reads as 500 and clamps to the 100K minimum"
    );
    assert_eq!(
        window(Some(OsStr::new("5000000")), None).tokens,
        1_000_000,
        "clamped to the 1M maximum"
    );
    assert_eq!(
        window(Some(OsStr::new("lots")), Some(300_000)),
        Window {
            tokens: 300_000,
            source: WindowSource::Model
        },
        "a value with no leading digits is ignored"
    );
}

#[test]
fn without_the_variable_the_model_window_then_200k_is_used() {
    assert_eq!(
        window(None, Some(1_000_000)),
        Window {
            tokens: 1_000_000,
            source: WindowSource::Model
        },
        "the model's context window"
    );
    assert_eq!(
        window(Some(OsStr::new("")), None),
        Window {
            tokens: 200_000,
            source: WindowSource::Default
        },
        "200,000 tokens when nothing says otherwise"
    );
}

#[test]
fn thresholds_are_absolute_tokens_of_the_window() {
    let window = Window {
        tokens: 200_000,
        source: WindowSource::Default,
    };
    assert_eq!(window.warn_at(), 120_000, "60% of 200,000");
    assert_eq!(window.stop_at(), 150_000, "75% of 200,000");
}

#[test]
fn the_level_is_none_below_60_warn_from_60_and_stop_from_75_percent() {
    let window = Window {
        tokens: 200_000,
        source: WindowSource::Default,
    };
    assert_eq!(window.level(119_999.0), None, "below the warning");
    assert_eq!(window.level(120_000.0), Some(Level::Warn), "at 60%");
    assert_eq!(window.level(149_999.5), Some(Level::Warn), "below 75%");
    assert_eq!(window.level(150_000.0), Some(Level::Stop), "at 75%");
}

#[test]
fn a_reading_measures_the_percentage_against_the_model_window() {
    let read = reading(Some(30.0), Some(1_000_000), Some(OsStr::new("400000")));

    assert_eq!(read.used_tokens, Some(300_000.0), "30% of 1M tokens");
    assert_eq!(
        read.window.level(read.used_tokens.unwrap_or_default()),
        Some(Level::Stop),
        "300k is past 75% of a 400k auto-compact window"
    );
}

#[test]
fn a_reading_without_a_usable_percentage_has_no_tokens() {
    for missing in [None, Some(-1.0), Some(f64::NAN)] {
        assert_eq!(
            reading(missing, Some(200_000), None).used_tokens,
            None,
            "{missing:?} is not a usage"
        );
    }
}

#[test]
fn a_percentage_over_100_reads_as_the_whole_window() {
    assert_eq!(
        reading(Some(250.0), Some(200_000), None).used_tokens,
        Some(200_000.0),
        "clamped to 100%"
    );
    assert_eq!(
        reading(Some(f64::MAX), Some(200_000), None).used_tokens,
        Some(200_000.0),
        "even the largest finite value"
    );
}

#[test]
fn each_threshold_fires_once_per_upward_crossing() {
    assert_eq!(crossing(None, None), None, "below: nothing");
    assert_eq!(
        crossing(None, Some(Level::Warn)),
        Some(Level::Warn),
        "crossing 60%"
    );
    assert_eq!(
        crossing(Some(Level::Warn), Some(Level::Warn)),
        None,
        "still above 60%: already fired"
    );
    assert_eq!(
        crossing(Some(Level::Warn), Some(Level::Stop)),
        Some(Level::Stop),
        "crossing 75%"
    );
    assert_eq!(
        crossing(Some(Level::Stop), Some(Level::Stop)),
        None,
        "still above 75%"
    );
    assert_eq!(
        crossing(Some(Level::Stop), Some(Level::Warn)),
        None,
        "falling back under 75% fires nothing"
    );
    assert_eq!(
        crossing(None, Some(Level::Stop)),
        Some(Level::Stop),
        "a jump past both fires the stop alone"
    );
}
