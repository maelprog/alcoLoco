//! The sampling grid of a series: its query parameters and its window.
//!
//! A series is sampled on a grid starting on a whole minute, `from`,
//! `from + step`, …, up to `to` included when it falls on the step. Every
//! instant of a curve is truncated to the minute: the bounds of the window,
//! the start of each drink and the instant of the current value. The step only
//! thins out the samples: every value is exact at its instant (SPEC.md §6.5),
//! so the step never changes a value, only how many are served (SPEC.md
//! §10.0-I, §10.3).
//!
//! **Cap per request.** One answer holds at most [`MAX_POINTS`] values, summed
//! over all its series. A step that would exceed it is not refused: it is
//! widened to the smallest multiple of a minute that fits, and the answer
//! carries the step actually used (SPEC.md §10.3, user decision of
//! 2026-10-21).

use utoipa::IntoParams;

use crate::error::FieldError;
use crate::timestamp::Timestamp;

/// Finest step a client may ask for, in seconds (SPEC.md §10.0-I).
pub const STEP_MIN_SECONDS: i64 = 60;

/// Coarsest step a client may ask for, in seconds (SPEC.md §10.0-I). The step
/// actually used may be coarser, when the cap of one answer requires it.
pub const STEP_MAX_SECONDS: i64 = 3600;

/// Step used when the request names none, in seconds (SPEC.md §10.3).
pub const DEFAULT_STEP_SECONDS: i64 = 60;

/// Decay period added after an event, in seconds (SPEC.md §10.0-I, §10.3).
pub const DECAY_SECONDS: i64 = 3 * 3600;

/// Seconds in a minute: the resolution of every instant of a curve, and the
/// unit a widened step is a multiple of.
pub const MINUTE_SECONDS: i64 = 60;

/// Most values one answer may hold, summed over its series: a week at the
/// finest step, both ends included.
pub const MAX_POINTS: i64 = 7 * 24 * 3600 / STEP_MIN_SECONDS + 1;

/// The `?from=&to=&step=` of `GET /profiles/{id}/bac`, exactly as they arrive.
///
/// Read as text, like [`crate::pagination::PageQuery`], so that an unreadable
/// member is a field error reported with everything else wrong with the
/// request.
#[derive(Debug, Default, Clone, serde::Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ProfileBacQuery {
    /// First instant of the series, mandatory. Truncated to the minute.
    #[param(value_type = String, format = DateTime, example = "2026-08-20T20:00:00Z")]
    pub from: Option<String>,
    /// Last instant of the series, mandatory, not before `from`. Truncated to
    /// the minute.
    #[param(value_type = String, format = DateTime, example = "2026-08-21T04:00:00Z")]
    pub to: Option<String>,
    /// Seconds between two samples, from 60 to 3600. Defaults to 60. Widened
    /// when the window would exceed the cap of one answer.
    #[param(value_type = Option<i64>, example = 60)]
    pub step: Option<String>,
}

/// The `?step=` of `GET /events/{id}/bac`, exactly as it arrives. The window
/// is the event's, never the client's.
#[derive(Debug, Default, Clone, serde::Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EventBacQuery {
    /// Seconds between two samples, from 60 to 3600. Defaults to 60. Widened
    /// when the participants and the window would exceed the cap of one
    /// answer.
    #[param(value_type = Option<i64>, example = 60)]
    pub step: Option<String>,
}

/// Instants sampled by a series, in seconds since the Unix epoch.
///
/// Empty when `to < from`: an open-ended event starting more than three hours
/// after the request has no window yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    /// First instant sampled.
    pub from: i64,
    /// Last instant of the window; sampled when it falls on the step.
    pub to: i64,
    /// Seconds between two samples, strictly positive.
    pub step: i64,
}

impl Grid {
    /// Number of instants sampled.
    #[must_use]
    pub fn len(&self) -> i64 {
        if self.to < self.from {
            0
        } else {
            (self.to - self.from) / self.step + 1
        }
    }

    /// Whether no instant is sampled.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The instants sampled, in order.
    #[must_use]
    pub fn times(&self) -> Vec<Timestamp> {
        (0..self.len())
            .filter_map(|index| instant(self.from + index * self.step))
            .collect()
    }

    /// The grid an answer of `series` series sampled on it may hold: itself
    /// when `len × series` is within [`MAX_POINTS`], otherwise the same
    /// window at the smallest multiple of a minute that fits. An answer
    /// without any series still serves the instants, and counts as one.
    ///
    /// When even a single sample per series exceeds the cap — more series
    /// than [`MAX_POINTS`] — the step spans the window, one sample each.
    #[must_use]
    pub fn fitted(self, series: usize) -> Self {
        let series = i64::try_from(series).unwrap_or(i64::MAX).max(1);
        if self.len().saturating_mul(series) <= MAX_POINTS {
            return self;
        }
        // `len` is then at least 2, so `to > from`. At most `per_series`
        // samples, i.e. `span / step ≤ per_series − 1`, which holds exactly
        // when `step > span / per_series`.
        let span = self.to - self.from;
        let per_series = (MAX_POINTS / series).max(1);
        let needed = span / per_series + 1;
        let step = needed
            .div_euclid(MINUTE_SECONDS)
            .saturating_add(i64::from(needed % MINUTE_SECONDS != 0))
            .saturating_mul(MINUTE_SECONDS);
        Self {
            step: step.max(self.step),
            ..self
        }
    }
}

/// The instant `seconds` after the Unix epoch.
#[must_use]
pub fn instant(seconds: i64) -> Option<Timestamp> {
    Timestamp::from_timestamp(seconds, 0)
}

/// `at` truncated to its minute, in seconds since the Unix epoch.
///
/// `timestamp` already rounds down to the second, the sub-second part of an
/// instant being never negative; `div_euclid` rounds down before the epoch
/// too.
#[must_use]
pub fn minute_of(at: Timestamp) -> i64 {
    at.timestamp().div_euclid(MINUTE_SECONDS) * MINUTE_SECONDS
}

/// Reads a step, reporting it under `step`; absent, it is the default.
fn read_step(errors: &mut Vec<FieldError>, step: Option<&str>) -> Option<i64> {
    let Some(text) = step else {
        return Some(DEFAULT_STEP_SECONDS);
    };
    // Digits only: `i64::from_str` also takes a leading `+`.
    let digits = !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    match text.parse::<i64>() {
        Ok(seconds) if digits && (STEP_MIN_SECONDS..=STEP_MAX_SECONDS).contains(&seconds) => {
            Some(seconds)
        }
        _ => {
            errors.push(FieldError::new(
                "step",
                format!(
                    "must be a whole number of seconds from {STEP_MIN_SECONDS} to {STEP_MAX_SECONDS}"
                ),
            ));
            None
        }
    }
}

/// Reads a mandatory instant, reporting it under `field`.
fn read_instant(
    errors: &mut Vec<FieldError>,
    field: &str,
    value: Option<&str>,
) -> Option<Timestamp> {
    let Some(text) = value else {
        errors.push(FieldError::new(field, "is required"));
        return None;
    };
    let at = text.parse::<Timestamp>().ok();
    if at.is_none() {
        errors.push(FieldError::new(
            field,
            "must be an RFC 3339 date-time, e.g. 2026-08-20T21:04:05Z",
        ));
    }
    at
}

impl ProfileBacQuery {
    /// Validates the window and the step, reporting every offending member at
    /// once, and fits the grid of one series within the cap.
    ///
    /// # Errors
    ///
    /// The field errors of each member that cannot be used.
    pub fn into_grid(self) -> Result<Grid, Vec<FieldError>> {
        let mut errors = Vec::new();
        let from = read_instant(&mut errors, "from", self.from.as_deref());
        let to = read_instant(&mut errors, "to", self.to.as_deref());
        let step = read_step(&mut errors, self.step.as_deref());

        if let (Some(from), Some(to)) = (from, to)
            && to < from
        {
            errors.push(FieldError::new("to", "must not be earlier than `from`"));
        }
        match (from, to, step) {
            (Some(from), Some(to), Some(step)) if errors.is_empty() => Ok(Grid {
                from: minute_of(from),
                to: minute_of(to),
                step,
            }
            .fitted(1)),
            _ => Err(errors),
        }
    }
}

impl EventBacQuery {
    /// The grid of an event of `participants` participants: from its start to
    /// three hours after its end — after the instant of the request for an
    /// open-ended event (SPEC.md §10.3, user decision of 2026-10-21). An event
    /// yet to start is sampled all the same, since its participants may have
    /// drunk before it; the grid is empty only when that end is before the
    /// start, i.e. for an open-ended event starting more than three hours
    /// after the request.
    ///
    /// # Errors
    ///
    /// The field errors of the step, unreadable or out of bounds.
    pub fn into_grid(
        self,
        starts_at: Timestamp,
        ends_at: Option<Timestamp>,
        now: Timestamp,
        participants: usize,
    ) -> Result<Grid, Vec<FieldError>> {
        let mut errors = Vec::new();
        let step = read_step(&mut errors, self.step.as_deref()).ok_or(errors)?;
        let end = ends_at.unwrap_or(now);
        Ok(Grid {
            from: minute_of(starts_at),
            to: minute_of(end).saturating_add(DECAY_SECONDS),
            step,
        }
        .fitted(participants))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().expect("a real instant")
    }

    fn seconds(text: &str) -> i64 {
        at(text).timestamp()
    }

    fn query(from: Option<&str>, to: Option<&str>, step: Option<&str>) -> ProfileBacQuery {
        ProfileBacQuery {
            from: from.map(str::to_owned),
            to: to.map(str::to_owned),
            step: step.map(str::to_owned),
        }
    }

    fn fields(errors: Vec<FieldError>) -> Vec<String> {
        errors.into_iter().map(|error| error.field).collect()
    }

    fn event(step: Option<&str>) -> EventBacQuery {
        EventBacQuery {
            step: step.map(str::to_owned),
        }
    }

    #[test]
    fn a_grid_samples_from_its_start_every_step_up_to_its_end() {
        let grid = Grid {
            from: 0,
            to: 150,
            step: 60,
        };
        assert_eq!(grid.len(), 3);
        assert_eq!(
            grid.times(),
            [instant(0), instant(60), instant(120)].map(Option::unwrap)
        );
        let exact = Grid { to: 120, ..grid };
        assert_eq!(exact.len(), 3);
        let single = Grid { to: 0, ..grid };
        assert_eq!(single.len(), 1);
        let empty = Grid { to: -1, ..grid };
        assert!(empty.is_empty());
        assert!(empty.times().is_empty());
    }

    #[test]
    fn an_instant_is_truncated_to_its_minute() {
        assert_eq!(minute_of(at("1970-01-01T00:01:59.9Z")), 60);
        assert_eq!(minute_of(at("1970-01-01T00:01:00Z")), 60);
        // Before the epoch, the minute below is the earlier one.
        assert_eq!(minute_of(at("1969-12-31T23:59:59.5Z")), -60);
        assert_eq!(minute_of(at("1969-12-31T23:59:00Z")), -60);
    }

    #[test]
    fn a_grid_within_the_cap_keeps_its_step() {
        let week = Grid {
            from: 0,
            to: 7 * 24 * 3600,
            step: 60,
        };
        assert_eq!(week.len(), MAX_POINTS);
        assert_eq!(week.fitted(1), week);
        assert_eq!(week.fitted(0), week, "no series counts as one");
        let evening = Grid {
            from: 0,
            to: 9 * 3600,
            step: 60,
        };
        assert_eq!(evening.fitted(10), evening);
        let odd = Grid {
            step: 90,
            ..evening
        };
        assert_eq!(odd.fitted(10), odd, "a step that fits is kept as asked");
    }

    #[test]
    fn a_grid_beyond_the_cap_takes_the_smallest_minute_multiple_that_fits() {
        for (grid, series) in [
            (
                Grid {
                    from: 0,
                    to: 7 * 24 * 3600 + 60,
                    step: 60,
                },
                1,
            ),
            (
                Grid {
                    from: 0,
                    to: 30 * 24 * 3600,
                    step: 60,
                },
                10,
            ),
            (
                Grid {
                    from: 0,
                    to: 9 * 3600,
                    step: 60,
                },
                30,
            ),
            (
                Grid {
                    from: 0,
                    to: 9 * 3600,
                    step: 90,
                },
                40,
            ),
            (
                Grid {
                    from: 120,
                    to: 400 * 24 * 3600 + 17,
                    step: 3600,
                },
                3,
            ),
        ] {
            let fitted = grid.fitted(series);
            let total = |grid: Grid| grid.len() * i64::try_from(series).expect("small");
            assert!(
                total(fitted) <= MAX_POINTS,
                "{grid:?} × {series}: {fitted:?}"
            );
            assert_eq!(fitted.step % MINUTE_SECONDS, 0, "{fitted:?}");
            assert!(fitted.step > grid.step, "{fitted:?}");
            assert_eq!((fitted.from, fitted.to), (grid.from, grid.to));
            // A minute less would not fit, unless it falls below what was
            // asked for, which did not fit either.
            let finer = Grid {
                step: fitted.step - MINUTE_SECONDS,
                ..grid
            };
            assert!(
                finer.step < grid.step || total(finer) > MAX_POINTS,
                "{grid:?} × {series}: {finer:?} fits too"
            );
        }
    }

    #[test]
    fn more_series_than_the_cap_get_one_sample_each() {
        let grid = Grid {
            from: 0,
            to: 3600,
            step: 60,
        };
        let fitted = grid.fitted(usize::try_from(MAX_POINTS).expect("small") + 1);
        assert_eq!(fitted.len(), 1);
    }

    #[test]
    fn a_profile_window_is_read_with_a_default_step() {
        let grid = query(
            Some("2026-08-20T20:00:00Z"),
            Some("2026-08-21T04:00:00Z"),
            None,
        )
        .into_grid()
        .expect("valid");
        assert_eq!(
            grid,
            Grid {
                from: seconds("2026-08-20T20:00:00Z"),
                to: seconds("2026-08-21T04:00:00Z"),
                step: DEFAULT_STEP_SECONDS,
            }
        );
    }

    #[test]
    fn the_bounds_of_a_profile_window_are_truncated_to_the_minute() {
        let grid = query(
            Some("2026-08-20T20:00:59.9Z"),
            Some("2026-08-21T04:00:30Z"),
            None,
        )
        .into_grid()
        .expect("valid");
        assert_eq!(grid.from, seconds("2026-08-20T20:00:00Z"));
        assert_eq!(grid.to, seconds("2026-08-21T04:00:00Z"));
        // Within one minute, one sample.
        let single = query(
            Some("2026-08-21T00:00:00.2Z"),
            Some("2026-08-21T00:00:40Z"),
            None,
        )
        .into_grid()
        .expect("valid");
        assert_eq!(single.len(), 1);
    }

    #[test]
    fn the_window_of_a_profile_is_mandatory_and_readable() {
        let missing = query(None, None, None).into_grid().expect_err("refused");
        assert_eq!(fields(missing), ["from", "to"]);
        let unreadable = query(Some("yesterday"), Some("2026-08-21"), Some("1 min"))
            .into_grid()
            .expect_err("refused");
        assert_eq!(fields(unreadable), ["from", "to", "step"]);
    }

    #[test]
    fn the_end_of_a_profile_window_is_not_before_its_start() {
        let reversed = query(
            Some("2026-08-21T00:00:00Z"),
            Some("2026-08-20T23:59:59Z"),
            None,
        )
        .into_grid()
        .expect_err("refused");
        assert_eq!(fields(reversed), ["to"]);
        let single = query(
            Some("2026-08-21T00:00:00Z"),
            Some("2026-08-21T00:00:00Z"),
            None,
        )
        .into_grid()
        .expect("valid");
        assert_eq!(single.len(), 1);
    }

    #[test]
    fn the_step_asked_for_is_held_to_one_minute_and_one_hour() {
        let window = |step: &str| {
            query(
                Some("2026-08-20T20:00:00Z"),
                Some("2026-08-21T04:00:00Z"),
                Some(step),
            )
            .into_grid()
        };
        assert_eq!(window("60").expect("the finest step").step, 60);
        assert_eq!(window("3600").expect("the coarsest step").step, 3600);
        for refused in [
            "59", "3601", "0", "-60", "60.5", "", " ", "1e2", "+60", "+3600",
        ] {
            let errors = window(refused).expect_err(refused);
            assert_eq!(fields(errors), ["step"], "step `{refused}`");
        }
        let refused = event(Some("30"))
            .into_grid(
                at("2026-08-20T20:00:00Z"),
                None,
                at("2026-08-20T21:00:00Z"),
                1,
            )
            .expect_err("refused");
        assert_eq!(fields(refused), ["step"]);
    }

    #[test]
    fn a_profile_window_beyond_a_week_at_one_minute_is_widened_not_refused() {
        let week = query(
            Some("2026-08-14T00:00:00Z"),
            Some("2026-08-21T00:00:00Z"),
            Some("60"),
        )
        .into_grid()
        .expect("a week at one minute");
        assert_eq!((week.len(), week.step), (MAX_POINTS, 60));
        let longer = query(
            Some("2026-08-14T00:00:00Z"),
            Some("2026-08-21T00:01:00Z"),
            Some("60"),
        )
        .into_grid()
        .expect("widened");
        assert_eq!(longer.step, 120);
    }

    #[test]
    fn a_past_event_is_sampled_until_three_hours_after_its_end() {
        let grid = event(None)
            .into_grid(
                at("2026-08-20T20:00:00Z"),
                Some(at("2026-08-21T02:00:00Z")),
                at("2026-10-21T12:00:00Z"),
                2,
            )
            .expect("valid");
        assert_eq!(
            grid,
            Grid {
                from: seconds("2026-08-20T20:00:00Z"),
                to: seconds("2026-08-21T05:00:00Z"),
                step: DEFAULT_STEP_SECONDS,
            }
        );
    }

    #[test]
    fn a_fixed_end_still_to_come_is_sampled_to_three_hours_after_it() {
        // User decision of 2026-10-21, "go to the end": no `min(now, end)`.
        let grid = event(None)
            .into_grid(
                at("2026-08-20T20:00:00Z"),
                Some(at("2026-08-21T02:00:00Z")),
                at("2026-08-20T22:30:00.5Z"),
                1,
            )
            .expect("valid");
        assert_eq!(grid.to, seconds("2026-08-21T05:00:00Z"));
    }

    #[test]
    fn an_open_ended_event_is_sampled_until_three_hours_after_now() {
        let grid = event(None)
            .into_grid(
                at("2026-08-20T20:00:00Z"),
                None,
                at("2026-08-20T22:30:40.5Z"),
                1,
            )
            .expect("valid");
        assert_eq!(grid.to, seconds("2026-08-21T01:30:00Z"));
    }

    #[test]
    fn an_event_yet_to_start_is_sampled_as_projected() {
        // User decision of 2026-10-21: the participants may have drunk before
        // the event. Starting in an hour and ending in five: up to its end
        // plus three hours. Open-ended, the end is now: still past the start.
        let now = at("2026-08-20T19:00:00.5Z");
        let starts_at = at("2026-08-20T20:00:00Z");
        let fixed = event(None)
            .into_grid(starts_at, Some(at("2026-08-21T00:00:00Z")), now, 1)
            .expect("valid");
        assert_eq!(fixed.from, seconds("2026-08-20T20:00:00Z"));
        assert_eq!(fixed.to, seconds("2026-08-21T03:00:00Z"));
        let open = event(None)
            .into_grid(starts_at, None, now, 1)
            .expect("valid");
        assert_eq!(open.to, seconds("2026-08-20T22:00:00Z"));
        assert_eq!(open.len(), 121);
        // Open-ended and starting more than three hours ahead: no sample yet.
        let later = event(None)
            .into_grid(at("2026-08-21T20:00:00Z"), None, now, 1)
            .expect("valid");
        assert!(later.is_empty());
    }

    #[test]
    fn many_participants_widen_the_step_of_an_event() {
        // 30 participants over 6 h + 3 h at one minute: 30 × 541 values,
        // above the cap. The step widens to two minutes: 30 × 271 fit.
        let grid = event(None)
            .into_grid(
                at("2026-08-20T20:00:00Z"),
                Some(at("2026-08-21T02:00:00Z")),
                at("2026-10-21T12:00:00Z"),
                30,
            )
            .expect("valid");
        assert_eq!(grid.step, 120);
        assert!(grid.len() * 30 <= MAX_POINTS);
        // Ten of them fit at one minute.
        let ten = event(None)
            .into_grid(
                at("2026-08-20T20:00:00Z"),
                Some(at("2026-08-21T02:00:00Z")),
                at("2026-10-21T12:00:00Z"),
                10,
            )
            .expect("valid");
        assert_eq!(ten.step, 60);
    }
}
