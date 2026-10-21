//! Blood alcohol curve of a profile: superposition of its drinks, drinking
//! episodes and display floor at zero (SPEC.md §6.3, §6.5).
//!
//! # The model
//!
//! The rates of appearance `Rᵢ` of the drinks add up; the elimination `β` is
//! global to the body and applies **once**, however many drinks are being
//! absorbed (§6.5). Summing single-drink curves that each carry their own
//! `− β × t` would eliminate at `N × β`. Each drink brings the `κᵢ = 0.806 /
//! TBWᵢ` of the body water in force at its own ingestion time (§5.6, §10.0-K):
//! the caller resolves it and hands it over in its [`Intake`].
//!
//! Drinks are grouped into **episodes**. Within an episode opened at `t₀` —
//! the start of its first drink — the curve is the closed form, without any
//! floor, with the elimination counted from `t₀`:
//!
//! ```text
//! raw(t) = Σᵢ κᵢ × absorbedᵢ(t − tᵢ) − β × (t − t₀)       C(t) = max(0, raw(t))
//! ```
//!
//! where `absorbedᵢ` is the primitive of `Rᵢ` given by the
//! [`AbsorptionProfile`]. Only the displayed value is floored: while the first
//! fractions of a drink appear slower than `β` eliminates, `raw` is negative and
//! the curve shows zero, and that elimination stays charged to the episode.
//!
//! An episode ends when the curve is at zero **and** no drink of it is still
//! being absorbed: past the last end of absorption `E`, `raw` only falls, so the
//! end is `max(E, t₀ + Σ κᵢ × Aᵢ / β)`. A drink starting at or after that end
//! opens a new episode: the elimination restarts from its own `t₀`, and no debt
//! is carried over (§6.3). A drink starting before it joins the episode, even
//! if the curve shows zero at that instant because a drink is still being
//! absorbed.
//!
//! Every value is exact at its instant — there is no integration step — so the
//! sampling step of a series never changes the result (§10.0-I). Drinks are
//! sorted by a total order first: same history, same profile, same `β` → same
//! curve, bit for bit, whatever the order of the slice (§7).

use std::cmp::Ordering;

use crate::absorption::{AbsorptionProfile, Dose};
use crate::{BodyWater, EngineError, initial_bac_g_per_l};

/// Seconds in an hour.
const SECONDS_PER_HOUR: f64 = 3600.0;

/// One drink of a profile, as the engine needs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Intake {
    /// Instant of the first sip, in seconds since the Unix epoch.
    pub started_at_seconds: i64,
    /// Its alcohol and durations.
    pub dose: Dose,
    /// Total body water of the profile **at that instant**, from the settings
    /// version in force and the age then (SPEC.md §5.6, §10.0-K).
    pub body_water: BodyWater,
}

/// One point of a curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BacSample {
    /// Instant, in seconds since the Unix epoch.
    pub at_seconds: i64,
    /// Blood alcohol concentration, in grams per litre, never negative.
    pub bac_g_per_l: f64,
}

/// Blood alcohol concentration, in grams per litre, at `at_seconds`.
///
/// `intakes` is the drink history of the profile. Only the episode under way
/// at `at_seconds` counts, so the history may be cut before the start of that
/// episode — before an instant where the curve was at zero **and** no drink
/// was still being absorbed — but not later. A drink starting exactly at
/// `at_seconds` is counted.
///
/// # Errors
///
/// [`EngineError::InvalidEliminationRate`] when `elimination_rate_g_per_l_per_h`
/// is not a finite, strictly positive number.
pub fn bac_at<P: AbsorptionProfile + ?Sized>(
    intakes: &[Intake],
    profile: &P,
    elimination_rate_g_per_l_per_h: f64,
    at_seconds: i64,
) -> Result<f64, EngineError> {
    let rate = check_elimination_rate(elimination_rate_g_per_l_per_h)?;
    let curve = Curve::new(intakes, profile, rate);
    Ok(curve.value_at(at_seconds))
}

/// Blood alcohol curve, sampled every `step_seconds` from `from_seconds` to
/// `to_seconds` included when it falls on the step.
///
/// Every sample is exactly [`bac_at`] at its instant. The bounds of the
/// `?step=` of the API, `[1 min, 1 h]`, and the window of SPEC.md §10.3 belong
/// to the caller.
///
/// # Errors
///
/// [`EngineError::InvalidEliminationRate`] as for [`bac_at`],
/// [`EngineError::NonPositiveStep`] when `step_seconds` is not positive and
/// [`EngineError::ReversedWindow`] when `to_seconds < from_seconds`.
pub fn bac_series<P: AbsorptionProfile + ?Sized>(
    intakes: &[Intake],
    profile: &P,
    elimination_rate_g_per_l_per_h: f64,
    from_seconds: i64,
    to_seconds: i64,
    step_seconds: i64,
) -> Result<Vec<BacSample>, EngineError> {
    let rate = check_elimination_rate(elimination_rate_g_per_l_per_h)?;
    if step_seconds <= 0 {
        return Err(EngineError::NonPositiveStep {
            seconds: step_seconds,
        });
    }
    if to_seconds < from_seconds {
        return Err(EngineError::ReversedWindow {
            from_seconds,
            to_seconds,
        });
    }
    let curve = Curve::new(intakes, profile, rate);
    let mut samples = Vec::new();
    let mut at = from_seconds;
    loop {
        samples.push(BacSample {
            at_seconds: at,
            bac_g_per_l: curve.value_at(at),
        });
        match at.checked_add(step_seconds) {
            Some(next) if next <= to_seconds => at = next,
            _ => return Ok(samples),
        }
    }
}

/// Refuses an elimination rate that is zero, negative or not finite.
fn check_elimination_rate(g_per_l_per_h: f64) -> Result<f64, EngineError> {
    if g_per_l_per_h.is_finite() && g_per_l_per_h > 0.0 {
        Ok(g_per_l_per_h)
    } else {
        Err(EngineError::InvalidEliminationRate { g_per_l_per_h })
    }
}

/// Hours in a number of seconds.
fn hours(seconds: i64) -> f64 {
    seconds as f64 / SECONDS_PER_HOUR
}

/// One drink, as the curve uses it.
struct Course {
    start: i64,
    dose: Dose,
    /// `κ = 0.806 / TBW`, of the body water at the start of the drink.
    kappa: f64,
    /// Elapsed hours after which the whole dose is absorbed.
    absorbed_after_hours: f64,
}

/// Total order on the drinks, so that the sums do not depend on the order of
/// the caller's slice: chronological first, then on every other input.
fn chronological(left: &Course, right: &Course) -> Ordering {
    left.start
        .cmp(&right.start)
        .then(
            left.dose
                .alcohol_grams()
                .total_cmp(&right.dose.alcohol_grams()),
        )
        .then(
            left.dose
                .ingestion_hours()
                .total_cmp(&right.dose.ingestion_hours()),
        )
        .then(
            left.dose
                .absorption_hours()
                .total_cmp(&right.dose.absorption_hours()),
        )
        .then(left.kappa.total_cmp(&right.kappa))
}

/// The drinks of one episode: `courses[first..end]`, opened at the start of
/// the first one.
struct Episode {
    first: usize,
    end: usize,
}

/// The drinks of a profile, sorted and split into episodes.
struct Curve<'a, P: AbsorptionProfile + ?Sized> {
    profile: &'a P,
    elimination_rate: f64,
    courses: Vec<Course>,
    episodes: Vec<Episode>,
}

impl<'a, P: AbsorptionProfile + ?Sized> Curve<'a, P> {
    fn new(intakes: &[Intake], profile: &'a P, elimination_rate: f64) -> Self {
        let mut courses: Vec<Course> = intakes
            .iter()
            .map(|intake| Course {
                start: intake.started_at_seconds,
                dose: intake.dose,
                kappa: initial_bac_g_per_l(1.0, intake.body_water),
                absorbed_after_hours: profile.fully_absorbed_after_hours(&intake.dose),
            })
            .collect();
        courses.sort_by(chronological);

        let mut episodes: Vec<Episode> = Vec::new();
        // Of the episode under way: its start, the concentration all its
        // drinks bring once absorbed, and its last end of absorption, in hours
        // after its start.
        let (mut opened_at, mut total, mut last_absorption) = (0_i64, 0.0_f64, 0.0_f64);
        for (index, course) in courses.iter().enumerate() {
            let since_opening = hours(course.start - opened_at);
            // Past `max(E, Σ κA / β)`, the curve is at zero and nothing is
            // being absorbed: the episode is over.
            let over = since_opening >= last_absorption.max(total / elimination_rate);
            match episodes.last_mut() {
                Some(episode) if !over => episode.end = index + 1,
                _ => {
                    episodes.push(Episode {
                        first: index,
                        end: index + 1,
                    });
                    (opened_at, total, last_absorption) = (course.start, 0.0, 0.0);
                }
            }
            total += course.kappa * course.dose.alcohol_grams();
            last_absorption =
                last_absorption.max(hours(course.start - opened_at) + course.absorbed_after_hours);
        }

        Self {
            profile,
            elimination_rate,
            courses,
            episodes,
        }
    }

    /// `max(0, raw(at))` in the last episode opened at or before `at`. Past
    /// its end, `raw` is negative, so no special case is needed.
    fn value_at(&self, at: i64) -> f64 {
        let opened = self
            .episodes
            .partition_point(|episode| self.courses[episode.first].start <= at);
        let Some(episode) = opened.checked_sub(1).map(|index| &self.episodes[index]) else {
            return 0.0;
        };
        let opened_at = self.courses[episode.first].start;
        let mut raw = 0.0;
        for course in &self.courses[episode.first..episode.end] {
            raw += course.kappa
                * self
                    .profile
                    .absorbed_grams(&course.dose, hours(at - course.start));
        }
        (raw - self.elimination_rate * hours(at - opened_at)).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::absorption::{PureWidmark, Trapezoid};
    use crate::{
        ELIMINATION_RATE_G_PER_L_PER_H, Sex, absorbed_alcohol_grams, ingested_alcohol_grams,
        initial_bac_g_per_l, total_body_water_litres, watson_total_body_water_litres,
    };

    // The reference case of SPEC.md §10.1: man, 80 kg, 180 cm, 30 years old;
    // 50 cL at 5 % vol (19.725 g); ingestion 20 min, absorption 30 min.
    const INGESTION_HOURS: f64 = 20.0 / 60.0;
    const ABSORPTION_HOURS: f64 = 30.0 / 60.0;
    const BETA: f64 = ELIMINATION_RATE_G_PER_L_PER_H;
    /// `C₀ = 0.806 × 19.725 / 45.8202`, pinned in `lib.rs`.
    const C0: f64 = 0.346_972_514_305_917_5;
    /// A whole minute: 29 333 334 minutes after the epoch.
    const T0: i64 = 1_760_000_040;
    const MINUTE: i64 = 60;
    const HOUR: i64 = 3600;

    /// Tolerance for values derived in `f64` by the test itself; the values
    /// compared are of order 1e-1 to 1e1.
    const DERIVED: f64 = 1e-12;

    fn grams() -> f64 {
        ingested_alcohol_grams(500.0, 5.0)
    }

    fn reference_body() -> BodyWater {
        watson_total_body_water_litres(Sex::Male, 80.0, 180.0, 30.0).expect("positive")
    }

    fn intake(at: i64, ingestion_hours: f64, absorption_hours: f64) -> Intake {
        Intake {
            started_at_seconds: at,
            dose: Dose::new(grams(), ingestion_hours, absorption_hours).expect("valid"),
            body_water: reference_body(),
        }
    }

    fn reference_drink(at: i64) -> Intake {
        intake(at, INGESTION_HOURS, ABSORPTION_HOURS)
    }

    fn at_once(at: i64) -> Intake {
        intake(at, 0.0, 0.0)
    }

    fn bac<P: AbsorptionProfile + ?Sized>(intakes: &[Intake], profile: &P, at: i64) -> f64 {
        bac_at(intakes, profile, BETA, at).expect("valid inputs")
    }

    /// Closed form of one drink alone, `κ × absorbed(τ) − β × τ`: the curve
    /// as an episode of that drink alone computes it, before the display floor.
    fn closed_form(drink: &Intake, at: i64) -> f64 {
        let tau = (at - drink.started_at_seconds) as f64 / 3600.0;
        let absorbed = absorbed_alcohol_grams(
            drink.dose.alcohol_grams(),
            drink.dose.ingestion_hours(),
            drink.dose.absorption_hours(),
            tau,
        )
        .expect("valid durations");
        initial_bac_g_per_l(absorbed, drink.body_water) - BETA * tau
    }

    // ----- one drink ---------------------------------------------------------

    #[test]
    fn a_drink_taken_at_once_falls_on_c0_minus_beta_t_and_stops_at_zero() {
        let drinks = [at_once(T0)];
        assert_eq!(bac(&drinks, &Trapezoid, T0 - HOUR), 0.0);
        assert_eq!(bac(&drinks, &Trapezoid, T0 - 1), 0.0);
        // C₀ / β = 138.789 min: positive up to minute 138, zero from 139 on.
        for minutes in [0, 1, 30, 60, 138] {
            let value = bac(&drinks, &Trapezoid, T0 + minutes * MINUTE);
            let expected = C0 - BETA * minutes as f64 / 60.0;
            assert!(
                (value - expected).abs() < DERIVED,
                "at {minutes} min: {value} instead of {expected}"
            );
        }
        for minutes in [139, 140, 600, 10_000] {
            assert_eq!(
                bac(&drinks, &Trapezoid, T0 + minutes * MINUTE),
                0.0,
                "at {minutes} min"
            );
        }
    }

    #[test]
    fn a_drink_between_two_whole_minutes_counts_from_its_own_start() {
        let start = T0 + 17;
        let drinks = [at_once(start)];
        for elapsed in [0, 1, 43, 60 + 43, 1800, 7000] {
            let value = bac(&drinks, &Trapezoid, start + elapsed);
            let expected = C0 - BETA * elapsed as f64 / 3600.0;
            assert!(
                (value - expected).abs() < DERIVED,
                "{elapsed} s after the drink: {value} instead of {expected}"
            );
        }
    }

    #[test]
    fn a_later_drink_between_two_whole_minutes_counts_from_its_own_start() {
        // The second drink starts 10 min 17 s after the first, while the first
        // is still in the blood: from its start on, the curve is 2 C₀ − β × t,
        // to the second.
        let second = T0 + 10 * MINUTE + 17;
        let drinks = [at_once(T0), at_once(second)];
        assert!(
            (bac(&drinks, &Trapezoid, second - 1)
                - (C0 - BETA * (second - 1 - T0) as f64 / 3600.0))
                .abs()
                < DERIVED
        );
        for elapsed in [0, 1, 43, 60 + 43, 1800] {
            let at = second + elapsed;
            let value = bac(&drinks, &Trapezoid, at);
            let expected = 2.0 * C0 - BETA * (at - T0) as f64 / 3600.0;
            assert!(
                (value - expected).abs() < DERIVED,
                "{elapsed} s after the second drink: {value} instead of {expected}"
            );
        }
    }

    #[test]
    fn the_zero_duration_trapezoid_is_pure_widmark() {
        // §6.4: a = b = 0 is instantaneous ingestion, whatever profile says so.
        let zero = [at_once(T0)];
        let durations = [reference_drink(T0)];
        let window = (T0 - HOUR, T0 + 4 * HOUR);
        let trapezoid =
            bac_series(&zero, &Trapezoid, BETA, window.0, window.1, MINUTE).expect("valid inputs");
        let widmark = bac_series(&durations, &PureWidmark, BETA, window.0, window.1, MINUTE)
            .expect("valid inputs");
        assert_eq!(trapezoid, widmark);
    }

    #[test]
    fn the_reference_trapezoid_peaks_at_0_227_g_per_l_after_45_7_minutes() {
        // SPEC.md §10.1, through the engine: within an episode the curve is the
        // closed form, elimination counted from the first sip, so its maximum
        // is where κ × R = β, τ* = 0.761 281 516 635 374 h = 2740.61 s, at
        // 0.227 376 400 558 264 47 g/L (both derived in `lib.rs`). Sampled
        // every second, the highest sample is the second nearest τ*.
        let drink = reference_drink(T0);
        let curve =
            bac_series(&[drink], &Trapezoid, BETA, T0, T0 + 2 * HOUR, 1).expect("valid inputs");
        let peak = curve
            .iter()
            .copied()
            .reduce(|best, sample| {
                if sample.bac_g_per_l > best.bac_g_per_l {
                    sample
                } else {
                    best
                }
            })
            .expect("a non-empty series");
        assert_eq!(
            peak.at_seconds - T0,
            2741,
            "peak at {}",
            peak.at_seconds - T0
        );
        assert!(
            (peak.bac_g_per_l - closed_form(&drink, peak.at_seconds)).abs() < DERIVED,
            "the peak is not on the closed form: {peak:?}"
        );
        assert!(
            peak.bac_g_per_l <= 0.227_376_400_558_264_47 + DERIVED
                && peak.bac_g_per_l > 0.227_376_400_558_264_47 - 1e-6,
            "{peak:?}"
        );
        // §10.1 quotes 0.227 g/L at 45.7 min.
        assert!(
            (peak.bac_g_per_l - 0.227).abs() < 5e-4,
            "disagrees with SPEC.md §10.1: {peak:?}"
        );
        assert!(
            ((peak.at_seconds - T0) as f64 / 60.0 - 45.7).abs() < 0.05,
            "disagrees with SPEC.md §10.1: {peak:?}"
        );
    }

    #[test]
    fn a_rectangular_profile_follows_its_closed_form_while_above_zero() {
        // §6.4: a single zero duration gives a constant rate A / 50 min, whose
        // κ × R = 0.416 g/L/h stays above β: the closed form is positive from
        // the first second to the return to zero.
        for drink in [intake(T0, 0.0, 50.0 / 60.0), intake(T0, 50.0 / 60.0, 0.0)] {
            for minutes in 1..=138 {
                let at = T0 + minutes * MINUTE;
                let value = bac(&[drink], &Trapezoid, at);
                let expected = closed_form(&drink, at);
                assert!(expected > 0.0, "{drink:?} at {minutes} min");
                assert!(
                    (value - expected).abs() < DERIVED,
                    "{drink:?} at {minutes} min: {value} instead of {expected}"
                );
            }
            // 0.347 − 0.15 × 140 / 60 < 0.
            assert_eq!(bac(&[drink], &Trapezoid, T0 + 140 * MINUTE), 0.0);
        }
    }

    #[test]
    fn a_single_drink_shows_its_closed_form_floored_at_zero() {
        // Within one episode, only the displayed value is floored: the curve
        // is max(0, κ × absorbed − β × τ) for every shape of §6.4 — zero while
        // the first fractions appear slower than β eliminates, the closed form
        // afterwards, zero again once eliminated.
        for (a, b) in [
            (INGESTION_HOURS, ABSORPTION_HOURS),
            (0.25, 0.25),
            (0.0, 50.0 / 60.0),
            (0.0, 0.0),
        ] {
            let drink = intake(T0, a, b);
            for minutes in 0..=240 {
                let at = T0 + minutes * MINUTE;
                let value = bac(&[drink], &Trapezoid, at);
                let expected = closed_form(&drink, at).max(0.0);
                assert!(
                    (value - expected).abs() < DERIVED,
                    "a = {a}, b = {b}, {minutes} min: {value} instead of {expected}"
                );
            }
        }
        // The early rise of the trapezoid is shown at zero: one minute in,
        // κ × A × τ² / (2ab) = 0.000 289 g/L, against 0.0025 g/L eliminated.
        let drink = reference_drink(T0);
        assert!(closed_form(&drink, T0 + MINUTE) < 0.0);
        assert_eq!(bac(&[drink], &Trapezoid, T0 + MINUTE), 0.0);
    }

    #[test]
    fn once_absorbed_every_shape_falls_on_c0_minus_beta_t() {
        // §10.1: the variants converge after absorption, on C₀ − β × t from the
        // first sip, down to zero, and stay there.
        for (a, b) in [
            (INGESTION_HOURS, ABSORPTION_HOURS),
            (0.25, 0.25),
            (0.0, 50.0 / 60.0),
            (0.0, 0.0),
        ] {
            let drink = intake(T0, a, b);
            let end = ((a + b) * 60.0).ceil() as i64;
            for minutes in end..=138 {
                let value = bac(&[drink], &Trapezoid, T0 + minutes * MINUTE);
                let expected = C0 - BETA * minutes as f64 / 60.0;
                assert!(
                    (value - expected).abs() < DERIVED,
                    "a = {a}, b = {b}, {minutes} min: {value} instead of {expected}"
                );
            }
            assert_eq!(bac(&[drink], &Trapezoid, T0 + 139 * MINUTE), 0.0);
            assert_eq!(bac(&[drink], &Trapezoid, T0 + 10 * HOUR), 0.0);
        }
    }

    // ----- several drinks: §6.5 ---------------------------------------------

    #[test]
    fn two_drinks_at_once_double_the_concentration_and_the_sobering_time() {
        // β is global to the body: 2 C₀ / β = 277.578 min. Applying it once
        // per drink would be back at zero after 138.8 min.
        let drinks = [at_once(T0), at_once(T0)];
        assert!((bac(&drinks, &Trapezoid, T0) - 2.0 * C0).abs() < DERIVED);
        let after_an_hour = bac(&drinks, &Trapezoid, T0 + HOUR);
        assert!(
            (after_an_hour - (2.0 * C0 - BETA)).abs() < DERIVED,
            "{after_an_hour}"
        );
        assert!(bac(&drinks, &Trapezoid, T0 + 277 * MINUTE) > 0.0);
        assert_eq!(bac(&drinks, &Trapezoid, T0 + 278 * MINUTE), 0.0);
    }

    #[test]
    fn overlapping_drinks_add_up_and_are_eliminated_once() {
        let first = reference_drink(T0);
        let second = reference_drink(T0 + 15 * MINUTE);
        let both = [first, second];
        for minutes in 0..=300 {
            let at = T0 + minutes * MINUTE;
            let together = bac(&both, &Trapezoid, at);
            assert!(together >= bac(&[first], &Trapezoid, at));
            assert!(together >= bac(&[second], &Trapezoid, at));
        }
        // Both absorbed 65 min after the first: from there the curve falls by
        // β per hour, not 2 β, down to zero at 2 C₀ / β = 277.6 min after the
        // first sip.
        for minutes in 65..277 {
            let now = bac(&both, &Trapezoid, T0 + minutes * MINUTE);
            let next = bac(&both, &Trapezoid, T0 + (minutes + 1) * MINUTE);
            assert!(
                (now - next - BETA / 60.0).abs() < DERIVED,
                "fell by {} at {minutes} min",
                now - next
            );
        }
        assert_eq!(bac(&both, &Trapezoid, T0 + 278 * MINUTE), 0.0);
    }

    #[test]
    fn a_drink_taken_while_another_is_absorbing_joins_its_episode() {
        // One minute after the first sip, the curve shows zero — the closed
        // form is negative — but the first drink is still being absorbed: the
        // episode is not over, and the second drink inherits the elimination
        // counted since the first sip.
        let first = reference_drink(T0);
        let second = at_once(T0 + MINUTE);
        assert_eq!(bac(&[first], &Trapezoid, T0 + MINUTE), 0.0);
        for minutes in 1..=200 {
            let at = T0 + minutes * MINUTE;
            let expected = (closed_form(&first, at) + C0).max(0.0);
            let value = bac(&[first, second], &Trapezoid, at);
            assert!(
                (value - expected).abs() < DERIVED,
                "{minutes} min: {value} instead of {expected}"
            );
        }
    }

    #[test]
    fn an_episode_that_never_rose_ends_with_its_absorption() {
        // 33 cL at 0.5 % drunk over two hours: κ × R stays far below β, the
        // closed form is negative throughout and the curve shows zero. Once the
        // dose is absorbed, the episode is over: a drink taken right after
        // starts afresh, with no debt.
        let mut light = intake(T0, 1.0, 1.0);
        light.dose = Dose::new(ingested_alcohol_grams(330.0, 0.5), 1.0, 1.0).expect("valid");
        let after = at_once(T0 + 2 * HOUR);
        for minutes in 0..=120 {
            assert_eq!(bac(&[light], &Trapezoid, T0 + minutes * MINUTE), 0.0);
        }
        for minutes in 0..=150 {
            let at = T0 + 2 * HOUR + minutes * MINUTE;
            assert_eq!(
                bac(&[light, after], &Trapezoid, at).to_bits(),
                bac(&[after], &Trapezoid, at).to_bits(),
                "{minutes} min after the second drink"
            );
        }
    }

    #[test]
    fn each_drink_uses_the_body_water_of_its_own_ingestion_time() {
        // §5.6: the settings in force at each ingestion time. A weight change
        // between two drinks changes the κ of the second one only.
        let lighter =
            watson_total_body_water_litres(Sex::Male, 70.0, 180.0, 30.0).expect("positive");
        let mut second = at_once(T0);
        second.body_water = lighter;
        let value = bac(&[at_once(T0), second], &Trapezoid, T0);
        let expected = C0 + initial_bac_g_per_l(grams(), lighter);
        assert!((value - expected).abs() < DERIVED, "{value} vs {expected}");
    }

    #[test]
    fn a_return_to_zero_carries_no_debt_to_the_next_drink() {
        // §6.3: once at zero with nothing left to absorb, the episode is over.
        // A drink taken five hours later yields exactly the curve it would
        // yield alone — bit for bit.
        let first = reference_drink(T0);
        let later = reference_drink(T0 + 5 * HOUR);
        assert_eq!(bac(&[first, later], &Trapezoid, T0 + 4 * HOUR), 0.0);
        for minutes in 0..=240 {
            let at = T0 + 5 * HOUR + minutes * MINUTE;
            assert_eq!(
                bac(&[first, later], &Trapezoid, at).to_bits(),
                bac(&[later], &Trapezoid, at).to_bits(),
                "{minutes} min after the second drink"
            );
        }
    }

    #[test]
    fn a_second_evening_is_drawn_as_if_it_were_alone() {
        // Two evenings of several overlapping drinks, three days apart: the
        // second one owes nothing to the first.
        let first_evening = an_evening();
        let shift = 3 * 24 * HOUR;
        let second_evening: Vec<Intake> = an_evening()
            .into_iter()
            .map(|mut drink| {
                drink.started_at_seconds += shift;
                drink
            })
            .collect();
        let both: Vec<Intake> = first_evening
            .iter()
            .chain(&second_evening)
            .copied()
            .collect();
        let (from, to) = (T0 + shift - HOUR, T0 + shift + 10 * HOUR);
        let together = bac_series(&both, &Trapezoid, BETA, from, to, MINUTE).expect("valid");
        let alone = bac_series(&second_evening, &Trapezoid, BETA, from, to, MINUTE).expect("valid");
        assert_eq!(together, alone);
        assert!(alone.iter().any(|sample| sample.bac_g_per_l > 0.2));
    }

    // ----- the body ---------------------------------------------------------

    #[test]
    fn the_same_drink_weighs_more_on_the_reference_woman() {
        // C₀ of `lib.rs`'s templates: 0.503 582… g/L for a woman of 65 kg /
        // 165 cm, against 0.346 972… g/L for the reference man.
        let woman =
            watson_total_body_water_litres(Sex::Female, 65.0, 165.0, 30.0).expect("positive");
        let mut drink = at_once(T0);
        drink.body_water = woman;
        let her = bac(&[drink], &Trapezoid, T0);
        let him = bac(&[at_once(T0)], &Trapezoid, T0);
        assert!((her - 0.503_582_458_307_597_3).abs() < DERIVED, "{her}");
        assert!((him - C0).abs() < DERIVED, "{him}");
    }

    #[test]
    fn the_widmark_fallback_flows_through_the_engine() {
        // No height: r = 0.68, and C₀ = A / (r × M) = 19.725 / 54.4.
        let mut drink = at_once(T0);
        drink.body_water =
            total_body_water_litres(Sex::Male, 80.0, None, Some(30.0)).expect("positive");
        let value = bac(&[drink], &Trapezoid, T0);
        assert!((value - grams() / (0.68 * 80.0)).abs() < DERIVED, "{value}");
    }

    // ----- determinism and sampling ------------------------------------------

    fn an_evening() -> Vec<Intake> {
        vec![
            reference_drink(T0 + 17),
            intake(T0 + 1234, 0.1, 0.75),
            at_once(T0 + 50 * MINUTE),
            reference_drink(T0 + 5 * HOUR + 7),
        ]
    }

    #[test]
    fn the_sampling_step_never_changes_the_result() {
        // §10.0-I: the step of a series only subsamples the integrated curve.
        let drinks = an_evening();
        let (from, to) = (T0 - 10 * MINUTE, T0 + 9 * HOUR);
        let by_second = bac_series(&drinks, &Trapezoid, BETA, from, to, 1).expect("valid inputs");
        for step in [7, MINUTE, 3 * MINUTE + 11, HOUR] {
            let series =
                bac_series(&drinks, &Trapezoid, BETA, from, to, step).expect("valid inputs");
            for sample in series {
                let index = usize::try_from(sample.at_seconds - from).expect("in the window");
                assert_eq!(
                    sample.bac_g_per_l.to_bits(),
                    by_second[index].bac_g_per_l.to_bits(),
                    "step {step}, at {}",
                    sample.at_seconds - T0
                );
            }
        }
        for sample in by_second.iter().step_by(97) {
            assert_eq!(
                sample.bac_g_per_l.to_bits(),
                bac(&drinks, &Trapezoid, sample.at_seconds).to_bits(),
                "at {}",
                sample.at_seconds - T0
            );
        }
    }

    #[test]
    fn the_same_history_gives_the_same_curve_whatever_its_order() {
        let drinks = an_evening();
        let mut reversed = drinks.clone();
        reversed.reverse();
        let once = bac_series(&drinks, &Trapezoid, BETA, T0, T0 + 9 * HOUR, MINUTE);
        let again = bac_series(&drinks, &Trapezoid, BETA, T0, T0 + 9 * HOUR, MINUTE);
        let other_order = bac_series(&reversed, &Trapezoid, BETA, T0, T0 + 9 * HOUR, MINUTE);
        assert_eq!(once, again);
        assert_eq!(once, other_order);
    }

    #[test]
    fn the_series_samples_its_window_at_the_requested_step() {
        let drinks = [reference_drink(T0)];
        let instants = |from, to, step| -> Vec<i64> {
            bac_series(&drinks, &Trapezoid, BETA, from, to, step)
                .expect("valid inputs")
                .into_iter()
                .map(|sample| sample.at_seconds - T0)
                .collect()
        };
        assert_eq!(
            instants(T0, T0 + 600, 120),
            vec![0, 120, 240, 360, 480, 600]
        );
        assert_eq!(
            instants(T0, T0 + 650, 120),
            vec![0, 120, 240, 360, 480, 600]
        );
        assert_eq!(instants(T0 + 5, T0 + 5, 60), vec![5]);
    }

    #[test]
    fn without_any_drink_the_curve_is_zero() {
        assert_eq!(bac(&[], &Trapezoid, T0), 0.0);
        let series = bac_series(&[], &Trapezoid, BETA, T0, T0 + HOUR, MINUTE).expect("valid");
        assert_eq!(series.len(), 61);
        assert!(series.iter().all(|sample| sample.bac_g_per_l == 0.0));
    }

    // ----- the profile is replaceable ---------------------------------------

    /// A profile written outside the engine: the alcohol appears at a constant
    /// rate over the whole `a + b`, the "linear ramp" variant of §10.1.
    struct LinearRamp;

    impl AbsorptionProfile for LinearRamp {
        fn absorbed_grams(&self, dose: &Dose, elapsed_hours: f64) -> f64 {
            let span = dose.ingestion_hours() + dose.absorption_hours();
            dose.alcohol_grams() * (elapsed_hours / span).clamp(0.0, 1.0)
        }

        fn fully_absorbed_after_hours(&self, dose: &Dose) -> f64 {
            dose.ingestion_hours() + dose.absorption_hours()
        }
    }

    #[test]
    fn a_new_profile_only_needs_an_implementation_of_the_trait() {
        // Same drinks, another model, no change to the engine: the ramp over
        // a + b is the a = 0 degeneracy of the trapezoid, so both must agree.
        let drinks = [reference_drink(T0), reference_drink(T0 + 20 * MINUTE)];
        let as_ramp = [
            intake(T0, 0.0, INGESTION_HOURS + ABSORPTION_HOURS),
            intake(T0 + 20 * MINUTE, 0.0, INGESTION_HOURS + ABSORPTION_HOURS),
        ];
        let window = (T0, T0 + 6 * HOUR);
        let ramp = bac_series(&drinks, &LinearRamp, BETA, window.0, window.1, MINUTE)
            .expect("valid inputs");
        let degenerate = bac_series(&as_ramp, &Trapezoid, BETA, window.0, window.1, MINUTE)
            .expect("valid inputs");
        for (left, right) in ramp.iter().zip(&degenerate) {
            assert!(
                (left.bac_g_per_l - right.bac_g_per_l).abs() < DERIVED,
                "{left:?} vs {right:?}"
            );
        }
        // And the trapezoid itself tells another story before its peak.
        let trapezoid = bac(&drinks, &Trapezoid, T0 + 20 * MINUTE);
        assert!((trapezoid - ramp[20].bac_g_per_l).abs() > 1e-3);
    }

    #[test]
    fn a_trait_object_is_accepted() {
        let profiles: [&dyn AbsorptionProfile; 2] = [&Trapezoid, &PureWidmark];
        for profile in profiles {
            assert!(bac(&[reference_drink(T0)], profile, T0 + HOUR) > 0.0);
        }
    }

    // ----- refusals ---------------------------------------------------------

    #[test]
    fn the_engine_refuses_a_meaningless_elimination_rate_step_or_window() {
        let drinks = [reference_drink(T0)];
        for rate in [0.0, -0.15, f64::NAN, f64::INFINITY] {
            assert!(
                matches!(
                    bac_at(&drinks, &Trapezoid, rate, T0),
                    Err(EngineError::InvalidEliminationRate { .. })
                ),
                "β = {rate}"
            );
            assert!(
                matches!(
                    bac_series(&drinks, &Trapezoid, rate, T0, T0 + HOUR, MINUTE),
                    Err(EngineError::InvalidEliminationRate { .. })
                ),
                "β = {rate}"
            );
        }
        for step in [0, -60] {
            assert_eq!(
                bac_series(&drinks, &Trapezoid, BETA, T0, T0 + HOUR, step),
                Err(EngineError::NonPositiveStep { seconds: step })
            );
        }
        assert_eq!(
            bac_series(&drinks, &Trapezoid, BETA, T0 + 1, T0, MINUTE),
            Err(EngineError::ReversedWindow {
                from_seconds: T0 + 1,
                to_seconds: T0
            })
        );
    }
}
