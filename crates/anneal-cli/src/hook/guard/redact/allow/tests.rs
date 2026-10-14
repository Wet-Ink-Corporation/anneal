use super::{
    EXACT_FLAGS, KEYED, NO_VALUE_FLAGS, SUBCOMMANDS, TARGETS, is_exact_flag, is_flag_cluster,
    is_key, is_option_name, is_program, is_subcommand, is_target,
};

#[test]
fn every_table_word_is_plain_short_ascii() {
    let words = TARGETS
        .iter()
        .chain(SUBCOMMANDS.iter().flat_map(|(_, words)| words.iter()))
        .chain(EXACT_FLAGS.iter().flat_map(|(_, flags)| flags.iter()))
        .chain(KEYED.iter().flat_map(|(_, keys)| keys.iter()));
    for word in words {
        assert!(
            (1..=32).contains(&word.len()) && word.bytes().all(|b| b.is_ascii_graphic()),
            "{word:?}"
        );
    }
}

#[test]
fn every_table_names_programs_the_skeleton_shows() {
    let programs = SUBCOMMANDS
        .iter()
        .map(|(program, _)| program)
        .chain(NO_VALUE_FLAGS.iter().map(|(program, _)| program))
        .chain(EXACT_FLAGS.iter().map(|(program, _)| program))
        .chain(KEYED.iter().map(|(program, _)| program));
    for program in programs {
        assert!(is_program(program), "{program:?}");
    }
}

#[test]
fn no_value_flags_are_ascii_letters() {
    for (program, flags) in NO_VALUE_FLAGS {
        assert!(
            !flags.is_empty() && flags.chars().all(|c| c.is_ascii_alphabetic()),
            "{program}: {flags:?}"
        );
    }
}

#[test]
fn a_program_word_is_a_short_name_of_safe_characters() {
    for kept in ["ls", "python3.12", "c++", "mkfs.ext4", "deploy.sh"] {
        assert!(is_program(kept), "{kept:?}");
    }
    let long = "a".repeat(65);
    for hidden in [
        "",
        "a b",
        "x=y",
        "$x",
        "a:b",
        "é",
        long.as_str(),
        "/usr/bin/python3.12",
        "./deploy.sh",
        "a/b",
    ] {
        assert!(!is_program(hidden), "{hidden:?}");
    }
}

#[test]
fn an_option_name_has_no_path_or_value_characters() {
    assert!(is_option_name("force-with-lease"));
    assert!(is_option_name("API_KEY"));
    for hidden in ["", "a/b", "a:b", "a b", "a+b"] {
        assert!(!is_option_name(hidden), "{hidden:?}");
    }
}

#[test]
fn block_devices_are_targets_and_lookalikes_are_not() {
    for device in [
        "/dev/sda",
        "/dev/sdb1",
        "/dev/xvdf",
        "/dev/vda12",
        "/dev/hdab3",
        "/dev/nvme0n1",
        "/dev/nvme10n2p3",
        "/dev/mmcblk0p1",
        "/dev/disk2s1",
        "/dev/rdisk4",
        "/dev/md0",
    ] {
        assert!(is_target(device), "{device:?}");
    }
    for other in [
        "/dev/sd",
        "/dev/sdabc",
        "/dev/sda1234",
        "/dev/sda-backup",
        "/dev/nvme0",
        "/dev/nvme0n1p",
        "/dev/disk",
        "/dev/md",
        "/dev/tty1",
        "/home/sda",
        "sda",
        "mainline",
        "HEAD~1",
    ] {
        assert!(!is_target(other), "{other:?}");
    }
}

#[test]
fn subcommands_keys_and_exact_flags_belong_to_their_program() {
    assert!(is_subcommand(Some("git"), "push"));
    assert!(!is_subcommand(Some("echo"), "push"));
    assert!(!is_subcommand(None, "push"));
    assert!(is_key(Some("dd"), "of"));
    assert!(!is_key(Some("cp"), "of"));
    assert!(is_exact_flag(Some("terraform"), "-auto-approve"));
    assert!(!is_exact_flag(Some("git"), "-auto-approve"));
}

#[test]
fn a_flag_cluster_is_kept_only_when_no_letter_can_take_a_value() {
    assert!(is_flag_cluster(None, "p"));
    assert!(is_flag_cluster(Some("rm"), "rf"));
    assert!(is_flag_cluster(Some("git"), "fdx"));
    assert!(!is_flag_cluster(Some("rm"), "rfz"));
    assert!(!is_flag_cluster(Some("mysql"), "psecret"));
    assert!(!is_flag_cluster(None, "rf"));
    assert!(!is_flag_cluster(None, ""));
    assert!(!is_flag_cluster(None, "é"));
}
