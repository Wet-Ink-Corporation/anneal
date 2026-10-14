//! Success claims: the vocabulary, and the negation, conditional, question and hyphen rules
//! that withdraw or keep a claim within its clause.

use super::success_claim;

#[test]
fn each_success_phrase_is_a_claim_in_any_letter_case() {
    let replies = [
        ("All Tests Pass now.", "all tests pass"),
        ("The tests pass.", "tests pass"),
        ("It works now!", "works now"),
        ("Done.", "done"),
        ("Everything is passing", "passing"),
        ("CI is GREEN", "green"),
        ("Fixed the parser.", "fixed"),
    ];
    for (reply, phrase) in replies {
        assert_eq!(success_claim(reply).as_deref(), Some(phrase), "{reply:?}");
    }
}

#[test]
fn inflections_and_other_finishing_words_are_claims_as_written() {
    let replies = [
        ("All tests passed.", "all tests passed"),
        ("the tests passed", "tests passed"),
        ("Everything passes.", "passes"),
        ("Tests: 40 passed, 0 failed.", "passed"),
        ("The implementation is complete.", "complete"),
        ("Migration completed", "completed"),
        ("Finished: everything works.", "finished"),
        ("The build succeeded.", "succeeded"),
        ("It succeeds on every target.", "succeeds"),
        ("All green.", "all green"),
        ("The test passes now", "test passes"),
    ];
    for (reply, claim) in replies {
        assert_eq!(success_claim(reply).as_deref(), Some(claim), "{reply:?}");
    }
}

#[test]
fn only_whole_words_count() {
    for reply in [
        "I abandoned the branch",
        "the greenfield rewrite",
        "a prefixed name",
        "undone work",
        "an incomplete list",
        "a password and a passport",
    ] {
        assert_eq!(success_claim(reply), None, "{reply:?}");
    }
}

#[test]
fn a_negation_reaches_only_within_its_clause() {
    for reply in [
        "No failures. Done.",
        "No warnings, no failures. Done.",
        "Nothing is left to change; done",
        "Not much changed: all tests pass",
        "It was not hard - fixed",
        "Never mind that\nDone",
    ] {
        assert!(success_claim(reply).is_some(), "{reply:?}");
    }
    for reply in ["not done", "It isn't fixed.", "The tests have not passed"] {
        assert_eq!(success_claim(reply), None, "{reply:?}");
    }
}

#[test]
fn words_inside_fenced_code_blocks_and_code_spans_are_not_claims() {
    let reply = "Here is the log:\n\n```\ntest result: ok. all tests pass\ndone\n```\n\n\
                 Run `cargo test --color green` to see it.";

    assert_eq!(success_claim(reply), None, "code is quoted, not claimed");
}

#[test]
fn a_negated_phrase_is_not_a_claim() {
    for reply in [
        "The tests are not passing yet.",
        "This isn't done.",
        "It is not yet fixed.",
        "Not all tests pass.",
    ] {
        assert_eq!(success_claim(reply), None, "{reply:?}");
    }
}

#[test]
fn finished_working_ready_and_suite_phrases_are_claims() {
    let replies = [
        ("Everything works.", "everything works"),
        ("The feature is implemented and working.", "implemented"),
        ("Ready to merge.", "ready to merge"),
        ("Resolved.", "resolved"),
        ("All checks pass.", "all checks pass"),
        ("The build passes", "build passes"),
        ("It's working", "it's working"),
        ("Everything is working now.", "everything is working"),
        ("Ready for review", "ready for review"),
    ];
    for (reply, claim) in replies {
        assert_eq!(success_claim(reply).as_deref(), Some(claim), "{reply:?}");
    }
}

#[test]
fn the_new_words_do_not_fire_on_ordinary_non_claims() {
    for reply in [
        "Does it work?",
        "Not implemented yet.",
        "Once checks pass, I'll merge.",
        "It isn't working.",
        "Tests should pass after the fix.",
        "Not everything works.",
        "It is not ready to merge.",
        "This isn't resolved",
    ] {
        assert_eq!(success_claim(reply), None, "{reply:?}");
    }
}

#[test]
fn a_question_is_not_a_claim() {
    for reply in [
        "Is everything working?",
        "Does it work now?",
        "Did the tests pass?",
        "Ready to merge?",
    ] {
        assert_eq!(success_claim(reply), None, "{reply:?}");
    }
    assert_eq!(
        success_claim("All tests pass - want me to commit?").as_deref(),
        Some("all tests pass"),
        "only the clause the question mark ends is a question"
    );
}

#[test]
fn a_conditional_before_a_claim_withdraws_it() {
    for reply in [
        "I'll merge once checks pass.",
        "If it works, I'll tag it.",
        "When the build passes, I'll tag it.",
        "Keep going until all tests pass",
        "Unless it's resolved upstream, we patch it.",
    ] {
        assert_eq!(success_claim(reply), None, "{reply:?}");
    }
    assert_eq!(
        success_claim("If it helps, all tests pass.").as_deref(),
        Some("all tests pass"),
        "a conditional reaches only within its clause"
    );
    assert_eq!(
        success_claim("All tests pass when run alone.").as_deref(),
        Some("all tests pass"),
        "a conditional after the claim does not withdraw it"
    );
}

#[test]
fn a_conditional_that_does_not_govern_the_claim_leaves_it() {
    let replies = [
        ("The crash when saving is fixed.", "fixed"),
        (
            "I ran the suite once more and all tests pass.",
            "all tests pass",
        ),
        ("The panic when the file is empty is resolved.", "resolved"),
    ];
    for (reply, claim) in replies {
        assert_eq!(success_claim(reply).as_deref(), Some(claim), "{reply:?}");
    }
}

#[test]
fn a_clause_opening_conditional_withdraws_every_claim_in_it() {
    for reply in [
        "When the build on every target passes",
        "Once the long-running suite passes",
    ] {
        assert_eq!(success_claim(reply), None, "{reply:?}");
    }
}

#[test]
fn a_hyphenated_suite_word_is_still_a_suite() {
    let replies = [
        ("All unit-tests pass.", "all unit-tests pass"),
        ("The e2e-tests pass", "e2e-tests pass"),
        ("smoke-checks pass", "smoke-checks pass"),
    ];
    for (reply, claim) in replies {
        assert_eq!(success_claim(reply).as_deref(), Some(claim), "{reply:?}");
    }
    assert_eq!(
        success_claim("Not all unit-tests pass."),
        None,
        "a negation still reaches a hyphenated suite claim"
    );
}

#[test]
fn one_adverb_between_subject_and_works_is_a_claim() {
    let replies = [
        ("Everything still works.", "everything still works"),
        ("It all works.", "it all works"),
        ("It now works.", "it now works"),
        ("It's still working", "it's still working"),
    ];
    for (reply, claim) in replies {
        assert_eq!(success_claim(reply).as_deref(), Some(claim), "{reply:?}");
    }
    for reply in ["Not everything still works.", "Does it still work?"] {
        assert_eq!(success_claim(reply), None, "{reply:?}");
    }
}

#[test]
fn a_question_reaches_only_the_clause_it_ends() {
    assert_eq!(
        success_claim("Did the tests pass, or should I re-run?").as_deref(),
        Some("tests pass"),
        "the comma ends the first clause before the question mark (accepted trade-off)"
    );
}

#[test]
fn an_in_word_hyphen_does_not_end_a_clause() {
    for reply in [
        "It's half-done, not finished.",
        "The parser is well-tested.",
        "A pre-fixed list",
        "Not re-done",
    ] {
        assert_eq!(success_claim(reply), None, "{reply:?}");
    }
}

#[test]
fn a_spaced_hyphen_double_hyphen_and_dashes_still_end_a_clause() {
    for reply in [
        "It was not hard - fixed",
        "It was not hard--fixed",
        "It was not hard -fixed",
        "It was not hard- fixed",
        "It was not hard \u{2013} fixed",
        "It was not hard\u{2014}fixed",
    ] {
        assert_eq!(success_claim(reply).as_deref(), Some("fixed"), "{reply:?}");
    }
}

#[test]
fn the_longest_phrase_is_reported() {
    assert_eq!(
        success_claim("ok, all tests pass and it works now").as_deref(),
        Some("all tests pass"),
        "the first phrase in list order"
    );
}

#[test]
fn a_bracketed_question_does_not_hide_the_claim_outside_it() {
    let replies = [
        ("Done (anything else?)", "done"),
        ("All tests pass (want me to push?)", "all tests pass"),
        (
            "Everything works now (should I also update the changelog?)",
            "works now",
        ),
        ("Fixed (let me know if you want a test?)", "fixed"),
        ("Fixed [see the log?]", "fixed"),
    ];
    for (reply, claim) in replies {
        assert_eq!(success_claim(reply).as_deref(), Some(claim), "{reply:?}");
    }
    assert_eq!(success_claim("(Is it done?)"), None, "the question itself");
}

#[test]
fn so_and_but_start_a_new_clause() {
    let replies = [
        ("All tests pass so should I merge?", "all tests pass"),
        ("It's fixed but should I add a test?", "fixed"),
        ("Not hard but fixed", "fixed"),
        // Accepted false positive: the question is only the clause after `so`.
        ("Is it done so I can merge?", "done"),
    ];
    for (reply, claim) in replies {
        assert_eq!(success_claim(reply).as_deref(), Some(claim), "{reply:?}");
    }
}

#[test]
fn up_to_two_filler_words_may_stand_between_a_claim_s_words() {
    let replies = [
        ("Everything is now working.", "everything is now working"),
        ("It is now working.", "it is now working"),
        (
            "Everything is still working.",
            "everything is still working",
        ),
        ("The tests now pass.", "tests now pass"),
        ("The tests all pass.", "tests all pass"),
        ("All the tests pass", "all the tests pass"),
        ("It finally really works.", "it finally really works"),
        (
            "Everything is also fully working",
            "everything is also fully working",
        ),
    ];
    for (reply, claim) in replies {
        assert_eq!(success_claim(reply).as_deref(), Some(claim), "{reply:?}");
    }
    for reply in [
        "The tests now really fully pass",
        "The tests do not all pass.",
        "Everything is not working",
        "Tests should pass after the fix.",
    ] {
        assert_eq!(success_claim(reply), None, "{reply:?}");
    }
}

#[test]
fn suites_joined_by_and_or_or_are_one_suite() {
    let replies = [
        ("Tests and lint pass.", "tests and lint pass"),
        ("All tests and checks pass", "all tests and checks pass"),
        ("The build or the tests pass", "tests pass"),
        ("Lint passes", "lint passes"),
    ];
    for (reply, claim) in replies {
        assert_eq!(success_claim(reply).as_deref(), Some(claim), "{reply:?}");
    }
    assert_eq!(success_claim("Not all tests and lint pass"), None);
}

#[test]
fn a_hyphenated_word_that_spells_a_whole_claim_is_that_claim() {
    let replies = [
        ("CI is all-green.", "all-green"),
        ("It's ready-to-merge.", "ready-to-merge"),
        ("PR is ready-for-review", "ready-for-review"),
    ];
    for (reply, claim) in replies {
        assert_eq!(success_claim(reply).as_deref(), Some(claim), "{reply:?}");
    }
    for reply in ["half-done", "not all-green", "a pre-fixed list"] {
        assert_eq!(success_claim(reply), None, "{reply:?}");
    }
}

#[test]
fn a_conditional_anywhere_earlier_in_the_clause_governs_the_claim() {
    for reply in [
        "I'll merge once the CI checks pass.",
        "I'll tag it when the release build passes.",
        "Wait until all the tests pass.",
        "Tag it when it is fixed.",
        "Merge it after everything is working.",
        "Ping me before the build passes",
    ] {
        assert_eq!(success_claim(reply), None, "{reply:?}");
    }
    let replies = [
        ("Once again all tests pass.", "all tests pass"),
        ("The bug when running locally is fixed.", "fixed"),
    ];
    for (reply, claim) in replies {
        assert_eq!(success_claim(reply).as_deref(), Some(claim), "{reply:?}");
    }
}

#[test]
fn a_purpose_word_before_a_works_claim_withdraws_it() {
    for reply in [
        "run the tests to make sure everything works.",
        "I still need to get it working.",
        "Let me know whether it works.",
        "Verify that everything is working",
        "Keep it working",
    ] {
        assert_eq!(success_claim(reply), None, "{reply:?}");
    }
    let replies = [
        ("I checked it twice and it works.", "it works"),
        ("To be sure, everything works.", "everything works"),
        ("I made sure; it works now.", "works now"),
        ("I switched to the new parser and it works.", "it works"),
    ];
    for (reply, claim) in replies {
        assert_eq!(success_claim(reply).as_deref(), Some(claim), "{reply:?}");
    }
}
