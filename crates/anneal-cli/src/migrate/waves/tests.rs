use anneal_ledger::LedgerEvent;
use anneal_ledger::event::{MigrateContract, MigrateWave, TaskDone};

use super::{Refusal, Step, Waves};

fn started(wave: u32) -> LedgerEvent {
    LedgerEvent::MigrateWaveStarted(MigrateWave {
        migration: "m".to_owned(),
        wave,
    })
}

fn done(wave: u32) -> LedgerEvent {
    LedgerEvent::MigrateWaveDone(MigrateWave {
        migration: "m".to_owned(),
        wave,
    })
}

fn contract() -> LedgerEvent {
    LedgerEvent::MigrateContract(MigrateContract {
        migration: "m".to_owned(),
    })
}

fn waves(events: &[LedgerEvent]) -> Waves {
    Waves::fold(events.iter())
}

#[test]
fn a_contract_while_a_wave_is_open_is_refused() {
    let waves = waves(&[started(1), done(1), started(2)]);

    assert_eq!(
        waves.check(Step::Contract),
        Err(Refusal::WavesOpen { waves: vec![2] }),
        "wave 2 started and is not done"
    );
}

#[test]
fn a_contract_after_every_wave_is_done_is_allowed() {
    let waves = waves(&[started(1), started(2), done(2), done(1)]);

    assert_eq!(waves.check(Step::Contract), Ok(()), "{waves:?}");
}

#[test]
fn a_contract_with_no_wave_done_is_refused() {
    assert_eq!(
        waves(&[]).check(Step::Contract),
        Err(Refusal::NoWaveDone),
        "nothing was migrated yet"
    );
}

#[test]
fn a_wave_starts_once_and_finishes_only_while_open() {
    let waves = waves(&[started(1), done(1), started(2)]);

    assert_eq!(
        [
            waves.check(Step::Start(1)),
            waves.check(Step::Start(2)),
            waves.check(Step::Done(1)),
            waves.check(Step::Done(3)),
            waves.check(Step::Start(3)),
            waves.check(Step::Done(2)),
        ],
        [
            Err(Refusal::AlreadyStarted { wave: 1 }),
            Err(Refusal::AlreadyStarted { wave: 2 }),
            Err(Refusal::NotOpen { wave: 1 }),
            Err(Refusal::NotOpen { wave: 3 }),
            Ok(()),
            Ok(()),
        ],
        "start once; done only an open wave"
    );
}

#[test]
fn nothing_follows_a_contract() {
    let waves = waves(&[started(1), done(1), contract()]);

    assert_eq!(
        [
            waves.check(Step::Start(2)),
            waves.check(Step::Done(1)),
            waves.check(Step::Contract),
        ],
        [
            Err(Refusal::Contracted),
            Err(Refusal::Contracted),
            Err(Refusal::Contracted)
        ],
        "a contracted migration is closed"
    );
}

#[test]
fn other_events_are_ignored() {
    let other = LedgerEvent::TaskDone(TaskDone {
        task: "m".to_owned(),
    });

    assert_eq!(
        waves(&[started(1), other, done(1)]).check(Step::Contract),
        Ok(()),
        "only migrate events count"
    );
}
