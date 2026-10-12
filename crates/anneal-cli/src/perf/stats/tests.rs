use super::{Comparison, MAX_RUNS, Samples, SamplesError, Verdict, t_975};

/// `SplitMix64`: a seeded, deterministic generator for test samples (no dependency, no clock).
struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A run time in seconds: `mean` plus noise of up to `spread` either way, in steps of
    /// 1 µs.
    fn run(&mut self, mean_micros: u32, spread_micros: u32) -> f64 {
        let width = u64::from(spread_micros) * 2 + 1;
        let offset = self.next() % width;
        let micros = u64::from(mean_micros - spread_micros) + offset;
        f64::from(u32::try_from(micros).expect("fits")) / 1e6
    }
}

fn samples(rng: &mut SplitMix, n: usize, mean_micros: u32, spread_micros: u32) -> Samples {
    Samples::new(
        (0..n)
            .map(|_| rng.run(mean_micros, spread_micros))
            .collect(),
    )
    .expect("valid samples")
}

#[test]
fn a_neutral_change_is_reverted() {
    // Baseline and candidate drawn from one distribution: no real delta.
    let mut rng = SplitMix(35);
    let baseline = samples(&mut rng, 20, 100_000, 5_000);
    let candidate = samples(&mut rng, 20, 100_000, 5_000);

    let comparison = Comparison::of(&baseline, &candidate);

    assert_eq!(comparison.verdict, Verdict::Neutral, "{comparison:?}");
    assert!(
        comparison.low < 0.0 && comparison.high > 0.0,
        "the interval holds 0: {comparison:?}"
    );
    assert!(!comparison.verdict.keeps(), "neutral means revert");
}

#[test]
fn a_neutral_change_is_kept_by_chance_at_most_at_the_intervals_error_rate() {
    // A 95% interval clears zero downwards by chance about 2.5% of the time; over 400 fixed
    // seeds (deterministic) no more than 5% may keep a change that did nothing.
    let seeds = 400_u64;
    let kept = (0..seeds)
        .filter(|seed| {
            let mut rng = SplitMix(*seed);
            let baseline = samples(&mut rng, 10, 50_000, 2_000);
            let candidate = samples(&mut rng, 10, 50_000, 2_000);
            Comparison::of(&baseline, &candidate).verdict.keeps()
        })
        .count();

    assert!(kept <= 20, "{kept} of {seeds} neutral changes kept");
}

#[test]
fn a_change_faster_beyond_the_interval_is_kept() {
    let mut rng = SplitMix(7);
    let baseline = samples(&mut rng, 10, 100_000, 2_000);
    let candidate = samples(&mut rng, 10, 90_000, 2_000);

    let comparison = Comparison::of(&baseline, &candidate);

    assert_eq!(comparison.verdict, Verdict::Faster, "{comparison:?}");
    assert!(comparison.high < 0.0, "the whole interval is below 0");
    assert!(comparison.verdict.keeps(), "only faster keeps");
}

#[test]
fn a_change_slower_beyond_the_interval_is_reverted() {
    let mut rng = SplitMix(8);
    let baseline = samples(&mut rng, 10, 90_000, 2_000);
    let candidate = samples(&mut rng, 10, 100_000, 2_000);

    let comparison = Comparison::of(&baseline, &candidate);

    assert_eq!(comparison.verdict, Verdict::Slower, "{comparison:?}");
    assert!(!comparison.verdict.keeps(), "a regression reverts");
}

#[test]
fn identical_constant_runs_are_neutral_and_distinct_ones_are_decisive() {
    let same = Samples::new(vec![0.5; 5]).expect("valid");
    let faster = Samples::new(vec![0.4; 5]).expect("valid");

    assert_eq!(
        (
            Comparison::of(&same, &same).verdict,
            Comparison::of(&same, &faster).verdict
        ),
        (Verdict::Neutral, Verdict::Faster),
        "zero variance: the interval is the delta itself"
    );
}

#[test]
fn the_t_value_is_the_tabled_one_at_or_below_the_degrees_of_freedom() {
    // Compared bit for bit: each expected value is a table entry, returned unchanged.
    assert_eq!(
        [0.5, 1.0, 1.9, 9.0, 30.5, 45.0, 1000.0].map(|df| t_975(df).to_bits()),
        [12.706, 12.706, 12.706, 2.262, 2.042, 2.021, 1.980].map(f64::to_bits),
        "rounded down to the next tabled df: never narrower than the exact interval"
    );
}

#[test]
fn samples_need_two_to_max_runs_finite_non_negative_values() {
    assert_eq!(
        Samples::new(vec![1.0]),
        Err(SamplesError::Count { count: 1 }),
        "one run has no variance"
    );
    assert_eq!(
        Samples::new(vec![1.0; MAX_RUNS + 1]),
        Err(SamplesError::Count {
            count: MAX_RUNS + 1
        }),
        "capped"
    );
    assert_eq!(
        Samples::new(vec![1.0, -1.0]),
        Err(SamplesError::Value { index: 1 }),
        "a negative time"
    );
    assert_eq!(
        Samples::new(vec![f64::INFINITY, 1.0]),
        Err(SamplesError::Value { index: 0 }),
        "an infinite time"
    );
}
