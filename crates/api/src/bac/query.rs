//! The sampling grid of a series: its query parameters and its window.
//!
//! A series is sampled on a grid of whole seconds, `from`, `from + step`, …,
//! up to `to` included when it falls on the step. The step only thins out the
//! samples: every value is exact at its instant (SPEC.md §6.5), so the step
//! never changes a value, only how many are served (SPEC.md §10.0-I, §10.3).

use utoipa::IntoParams;

use crate::error::FieldError;
use crate::timestamp::Timestamp;

/// Finest step a client may ask for, in seconds (SPEC.md §10.0-I).
pub const STEP_MIN_SECONDS: i64 = 60;

/// Coarsest step a client may ask for, in seconds (SPEC.md §10.0-I).
pub const STEP_MAX_SECONDS: i64 = 3600;

/// Step used when the request names none, in seconds (SPEC.md §10.3).
pub const DEFAULT_STEP_SECONDS: i64 = 60;

/// Decay period added after an event, in seconds (SPEC.md §10.0-I, §10.3).
pub const DECAY_SECONDS: i64 = 3 * 3600;

/// Most samples one series may hold: a week at the finest step, both ends
/// included. The bounds of the step alone do not bound a response, since the
/// window of a profile is the client's; this does.
pub const MAX_SAMPLES: i64 = 7 * 24 * 3600 / STEP_MIN_SECONDS + 1;

/// The `?from=&to=&step=` of `GET /profiles/{id}/bac`, exactly as they arrive.
///
/// Read as text, like [`crate::pagination::PageQuery`], so that an unreadable
/// member is a field error reported with everything else wrong with the
/// request.
#[derive(Debug, Default, Clone, serde::Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ProfileBacQuery {
    /// First instant of the series, mandatory. Rounded up to the second.
    #[param(value_type = String, format = DateTime, example = "2026-08-20T20:00:00Z")]
    pub from: Option<String>,
    /// Last instant of the series, mandatory, not before `from`. Rounded down
    /// to the second.
    #[param(value_type = String, format = DateTime, example = "2026-08-21T04:00:00Z")]
    pub to: Option<String>,
    /// Seconds between two samples, from 60 to 3600. Defaults to 60.
    #[param(value_type = Option<i64>, example = 60)]
    pub step: Option<String>,
}

/// The `?step=` of `GET /events/{id}/bac`, exactly as it arrives. The window
/// is the event's, never the client's.
#[derive(Debug, Default, Clone, serde::Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EventBacQuery {
    /// Seconds between two samples, from 60 to 3600. Defaults to 60.
    #[param(value_type = Option<i64>, example = 60)]
    pub step: Option<String>,
}

/// Instants sampled by a series, in seconds since the Unix epoch.
///
/// Empty when `to < from`: an event that has not started yet has no window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    /// First instant sampled.
    pub from: i64,
    /// Last instant of the window; sampled when it falls on the step.
    pub to: i64,
    /// Seconds between two samples, within the bounds of the step.
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
}

/// The instant `seconds` after the Unix epoch.
#[must_use]
pub fn instant(seconds: i64) -> Option<Timestamp> {
    Timestamp::from_timestamp(seconds, 0)
}

/// The first whole second at or after `at`.
#[must_use]
pub fn second_at_or_after(at: Timestamp) -> i64 {
    at.timestamp() + i64::from(at.timestamp_subsec_nanos() > 0)
}

/// The last whole second at or before `at`.
///
/// `timestamp` already rounds down: the sub-second part of an instant is
/// never negative, before the epoch too.
#[must_use]
pub fn second_at_or_before(at: Timestamp) -> i64 {
    at.timestamp()
}

/// Reads a step, reporting it under `step`; absent, it is the default.
fn read_step(errors: &mut Vec<FieldError>, step: Option<&str>) -> Option<i64> {
    let Some(text) = step else {
        return Some(DEFAULT_STEP_SECONDS);
    };
    match text.parse::<i64>() {
        Ok(seconds) if (STEP_MIN_SECONDS..=STEP_MAX_SECONDS).contains(&seconds) => Some(seconds),
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

/// Refuses a grid holding more than [`MAX_SAMPLES`] instants, naming the
/// finest step that would fit.
fn bounded(errors: &mut Vec<FieldError>, grid: Grid) -> Option<Grid> {
    if grid.len() <= MAX_SAMPLES {
        return Some(grid);
    }
    let span = grid.to - grid.from;
    let finest = span / (MAX_SAMPLES - 1) + i64::from(span % (MAX_SAMPLES - 1) != 0);
    errors.push(FieldError::new(
        "step",
        format!(
            "gives {} samples, more than the {MAX_SAMPLES} a series may hold: \
             take a step of at least {finest} seconds, or a shorter window",
            grid.len()
        ),
    ));
    None
}

impl ProfileBacQuery {
    /// Validates the window and the step, reporting every offending member at
    /// once.
    ///
    /// # Errors
    ///
    /// The field errors of each member that cannot be used.
    pub fn into_grid(self) -> Result<Grid, Vec<FieldError>> {
        let mut errors = Vec::new();
        let from = read_instant(&mut errors, "from", self.from.as_deref());
        let to = read_instant(&mut errors, "to", self.to.as_deref());
        let step = read_step(&mut errors, self.step.as_deref());

        let window = match (from, to) {
            (Some(from), Some(to)) => {
                let (from, to) = (second_at_or_after(from), second_at_or_before(to));
                if to < from {
                    errors.push(FieldError::new(
                        "to",
                        "must not be earlier than `from`, counted in whole seconds",
                    ));
                    None
                } else {
                    Some((from, to))
                }
            }
            _ => None,
        };
        let grid = match (window, step) {
            (Some((from, to)), Some(step)) => bounded(&mut errors, Grid { from, to, step }),
            _ => None,
        };
        match grid {
            Some(grid) if errors.is_empty() => Ok(grid),
            _ => Err(errors),
        }
    }
}

impl EventBacQuery {
    /// The grid of an event: from its start to `min(now, end) + 3 h`, an
    /// open-ended event ending now (SPEC.md §10.3). Empty while the event has
    /// not started, whatever its end: `to` is then the instant of the request,
    /// earlier than `from` — even for an event starting within three hours,
    /// where `now + 3 h` would otherwise reach past its start.
    ///
    /// # Errors
    ///
    /// The field errors of the step, unreadable or out of bounds, or too fine
    /// for the window of the event.
    pub fn into_grid(
        self,
        starts_at: Timestamp,
        ends_at: Option<Timestamp>,
        now: Timestamp,
    ) -> Result<Grid, Vec<FieldError>> {
        let mut errors = Vec::new();
        let step = read_step(&mut errors, self.step.as_deref());
        let end = ends_at.map_or(now, |ends_at| ends_at.min(now));
        let from = second_at_or_after(starts_at);
        let to = if now < starts_at {
            second_at_or_before(now)
        } else {
            second_at_or_before(end).saturating_add(DECAY_SECONDS)
        };
        match step.and_then(|step| bounded(&mut errors, Grid { from, to, step })) {
            Some(grid) => Ok(grid),
            None => Err(errors),
        }
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
    fn an_instant_is_rounded_inwards_to_the_second() {
        assert_eq!(second_at_or_after(at("1970-01-01T00:00:01.2Z")), 2);
        assert_eq!(second_at_or_before(at("1970-01-01T00:00:01.8Z")), 1);
        assert_eq!(second_at_or_after(at("1970-01-01T00:00:01Z")), 1);
        assert_eq!(second_at_or_before(at("1970-01-01T00:00:01Z")), 1);
        // Before the epoch, the second below is the earlier one.
        assert_eq!(second_at_or_before(at("1969-12-31T23:59:59.5Z")), -1);
        assert_eq!(second_at_or_after(at("1969-12-31T23:59:59.5Z")), 0);
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
        // Within one second, rounding inwards leaves no whole second.
        let no_second = query(
            Some("2026-08-21T00:00:00.2Z"),
            Some("2026-08-21T00:00:00.7Z"),
            None,
        )
        .into_grid()
        .expect_err("refused");
        assert_eq!(fields(no_second), ["to"]);
        // A window of one instant is one sample.
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
    fn the_step_is_held_to_one_minute_and_one_hour() {
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
        for refused in ["59", "3601", "0", "-60", "60.5", "", " ", "1e2"] {
            let errors = window(refused).expect_err(refused);
            assert_eq!(fields(errors), ["step"], "step `{refused}`");
        }
    }

    #[test]
    fn a_profile_window_holds_at_most_a_week_at_the_finest_step() {
        let week = query(
            Some("2026-08-14T00:00:00Z"),
            Some("2026-08-21T00:00:00Z"),
            Some("60"),
        )
        .into_grid()
        .expect("a week at one minute");
        assert_eq!(week.len(), MAX_SAMPLES);
        let longer = query(
            Some("2026-08-14T00:00:00Z"),
            Some("2026-08-21T00:01:00Z"),
            Some("60"),
        )
        .into_grid()
        .expect_err("refused");
        assert_eq!(fields(longer), ["step"]);
        // The same window at a coarser step holds fewer samples.
        query(
            Some("2026-08-14T00:00:00Z"),
            Some("2026-08-21T00:01:00Z"),
            Some("120"),
        )
        .into_grid()
        .expect("a week at two minutes");
    }

    #[test]
    fn a_past_event_is_sampled_until_three_hours_after_its_end() {
        let grid = EventBacQuery::default()
            .into_grid(
                at("2026-08-20T20:00:00Z"),
                Some(at("2026-08-21T02:00:00Z")),
                at("2026-10-21T12:00:00Z"),
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
    fn an_event_under_way_is_sampled_until_three_hours_after_now() {
        let now = at("2026-08-20T22:30:00.5Z");
        let expected = seconds("2026-08-21T01:30:00Z");
        let open = EventBacQuery::default()
            .into_grid(at("2026-08-20T20:00:00Z"), None, now)
            .expect("valid");
        assert_eq!(open.to, expected);
        let ending_later = EventBacQuery::default()
            .into_grid(
                at("2026-08-20T20:00:00Z"),
                Some(at("2026-08-21T02:00:00Z")),
                now,
            )
            .expect("valid");
        assert_eq!(ending_later.to, expected);
    }

    #[test]
    fn an_event_not_started_has_no_sample() {
        let grid = EventBacQuery::default()
            .into_grid(at("2026-08-21T20:00:00Z"), None, at("2026-08-20T12:00:00Z"))
            .expect("valid");
        assert!(grid.is_empty());
    }

    #[test]
    fn an_event_starting_within_the_decay_period_has_no_sample_yet() {
        // Starting in an hour, `min(now, end) + 3 h` is already past the
        // start: the window must still be empty until the event begins, and
        // end at the instant of the request, whatever the end of the event.
        let now = at("2026-08-20T19:00:00.5Z");
        for ends_at in [None, Some(at("2026-08-21T02:00:00Z"))] {
            let grid = EventBacQuery::default()
                .into_grid(at("2026-08-20T20:00:00Z"), ends_at, now)
                .expect("valid");
            assert!(grid.is_empty(), "{ends_at:?}: {grid:?}");
            assert_eq!(grid.to, seconds("2026-08-20T19:00:00Z"));
        }
        // Started half a second before the request: one sample, its start.
        let grid = EventBacQuery::default()
            .into_grid(at("2026-08-20T19:00:00Z"), None, now)
            .expect("valid");
        assert_eq!(grid.times().first(), Some(&at("2026-08-20T19:00:00Z")));
    }

    #[test]
    fn the_step_of_an_event_is_held_to_its_bounds_and_to_the_cap() {
        let event = |step: &str, ends_at: &str| {
            EventBacQuery {
                step: Some(step.to_owned()),
            }
            .into_grid(
                at("2026-08-01T00:00:00Z"),
                Some(at(ends_at)),
                at("2026-10-21T12:00:00Z"),
            )
        };
        assert_eq!(
            fields(event("30", "2026-08-01T06:00:00Z").expect_err("refused")),
            ["step"]
        );
        // Thirty days and three hours do not fit a week at one minute.
        assert_eq!(
            fields(event("60", "2026-08-31T00:00:00Z").expect_err("refused")),
            ["step"]
        );
        event("600", "2026-08-31T00:00:00Z").expect("ten minutes fit");
    }
}
