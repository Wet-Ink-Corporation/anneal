//! `names_settings`: the names Windows reads as `.anneal/config.json`, and near misses.

use std::path::Path;

use super::names_settings;

#[test]
fn names_windows_reads_as_the_settings_file_are_the_settings_file() {
    let rows = [
        "w/.anneal/config.json",
        "w/.anneal/config.json.",
        "w/.anneal/config.json ",
        "w/.anneal/config.json. .",
        "w/.anneal/config.json::$DATA",
        "w/.anneal/config.json:stream",
        "w/.anneal./config.json",
        "w/.ANNEAL/Config.JSON",
    ];

    let missed: Vec<&str> = rows
        .into_iter()
        .filter(|path| !names_settings(Path::new(path)))
        .collect();

    assert_eq!(missed, Vec::<&str>::new(), "each names the settings file");
}

#[test]
fn near_misses_of_the_settings_file_are_not_it() {
    let rows = [
        "w/.anneal/config.json.bak",
        "w/.anneal/config.jsonx",
        "w/.anneal/xconfig.json",
        "w/anneal/config.json",
        "w/.anneal/briefs/config.json",
        "config.json",
    ];

    let matched: Vec<&str> = rows
        .into_iter()
        .filter(|path| names_settings(Path::new(path)))
        .collect();

    assert_eq!(matched, Vec::<&str>::new(), "none names the settings file");
}
