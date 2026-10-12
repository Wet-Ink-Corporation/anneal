use super::{Comparison, MAX_RUNS, Samples, SamplesError, T_975, Verdict, t_975};

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
fn a_worked_example_gives_the_hand_computed_delta_df_t_and_interval() {
    // Baseline [1, 2, 3, 4]: mean 5/2, variance 5/3. Candidate [2, 4, 6, 8, 10]: mean 6,
    // variance 10. Squared standard error 10/5 + (5/3)/4 = 29/12. Welch df
    // (29/12)² / ((10/5)² / 4 + (5/12)² / 3) = 2523/457 ≈ 5.52, so the df 5 row: t = 2.571.
    // Half-width 2.571 * sqrt(29/12) = 2.571 * 1.5545631755148024.
    let baseline = Samples::new(vec![1.0, 2.0, 3.0, 4.0]).expect("valid");
    let candidate = Samples::new(vec![2.0, 4.0, 6.0, 8.0, 10.0]).expect("valid");
    let half = 3.996_781_924_248_557;

    let comparison = Comparison::of(&baseline, &candidate);

    let close = |got: f64, want: f64| (got - want).abs() < 1e-9;
    assert!(
        close(comparison.delta, 3.5)
            && close(comparison.df, 2523.0 / 457.0)
            && close(comparison.t, 2.571)
            && close(comparison.low, 3.5 - half)
            && close(comparison.high, 3.5 + half),
        "{comparison:?}"
    );
    assert_eq!(comparison.verdict, Verdict::Neutral, "the interval holds 0");
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
        [12.707, 12.707, 12.707, 2.263, 2.043, 2.022, 1.980].map(f64::to_bits),
        "rounded down to the next tabled df: never narrower than the exact interval"
    );
}

/// `∫ cos^(df-1) θ dθ` from 0 to `upper`, by Simpson's rule on 4,000 intervals.
///
/// With `x = sqrt(df) tan θ`, Student's t density on `[0, x]` becomes `cos^(df-1) θ` on
/// `[0, atan(x / sqrt(df))]`, up to a constant, so the share of `[0, π/2)` below `θ` is the
/// share of the positive half of the distribution below `x`.
fn cos_power_integral(df: u32, upper: f64) -> f64 {
    let power = i32::try_from(df).expect("a tabled df") - 1;
    let steps = 4_000_u32;
    let h = upper / f64::from(steps);
    let f = |i: u32| (f64::from(i) * h).cos().powi(power);
    let inner: f64 = (1..steps)
        .map(|i| if i % 2 == 1 { 4.0 * f(i) } else { 2.0 * f(i) })
        .sum();
    (f(0) + inner + f(steps)) * h / 3.0
}

/// The exact 97.5th percentile of Student's t with `df` degrees of freedom, to about 1e-9:
/// the `x` whose `θ` holds 95% of the positive half (0.475 of the whole).
fn exact_t_975(df: u32) -> f64 {
    let whole = cos_power_integral(df, std::f64::consts::FRAC_PI_2);
    let (mut low, mut high) = (0.0, std::f64::consts::FRAC_PI_2);
    for _ in 0..60 {
        let mid = f64::midpoint(low, high);
        if cos_power_integral(df, mid) / whole < 0.95 {
            low = mid;
        } else {
            high = mid;
        }
    }
    f64::from(df).sqrt() * f64::midpoint(low, high).tan()
}

#[test]
fn the_exact_quantile_is_computed_correctly_where_it_is_known_in_closed_form() {
    // df 1 is Cauchy: t = tan(π (0.975 - 1/2)). df 2 has CDF 1/2 + t / (2 sqrt(2 + t²)), so
    // t / sqrt(2 + t²) = 0.95 and t² = 2 * 0.9025 / 0.0975.
    let (one, two) = (exact_t_975(1), exact_t_975(2));
    let cauchy = (0.475 * std::f64::consts::PI).tan();
    let df_2 = (2.0 * 0.9025_f64 / 0.0975).sqrt();

    assert!(
        (one - cauchy).abs() < 1e-9 && (two - df_2).abs() < 1e-9,
        "{one} vs {cauchy}, {two} vs {df_2}"
    );
}

#[test]
fn every_tabled_t_is_at_or_above_the_exact_one_and_within_0_001_of_it() {
    // At or above: rounding df down then picks a t no smaller than the exact one, so the
    // interval is never narrower than the exact interval.
    let off: Vec<(u32, f64, f64)> = T_975
        .iter()
        .map(|&(df, tabled)| (df, tabled, exact_t_975(df)))
        .filter(|&(_, tabled, exact)| !(tabled >= exact && tabled - exact < 0.001))
        .collect();

    assert_eq!(
        off,
        [],
        "(df, tabled, exact) rows not rounded up to 3 decimals"
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
