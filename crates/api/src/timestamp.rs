//! The one date-time type crossing the API.
//!
//! Timestamps are stored as `timestamptz` and travel as ISO 8601 in UTC with the
//! `Z` suffix. No local time ever crosses the API: converting to the reader's
//! zone is the front-end's job (SPEC.md §7). Naming the type here rather than
//! spelling `DateTime<Utc>` in every payload is what makes that rule checkable —
//! a payload reaching for a zoned or naive type would have to bypass this alias.

/// Every instant exchanged by the API.
pub type Timestamp = chrono::DateTime<chrono::Utc>;

/// The current instant, in the only form the API exchanges.
#[must_use]
pub fn now() -> Timestamp {
    chrono::Utc::now()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timestamp_serialises_as_iso_8601_utc_with_a_z_suffix() {
        // Guards the choice of type behind the alias: a local or offset-carrying
        // type would render `+02:00` (or `+00:00`) instead of `Z` and fail here.
        let rendered = serde_json::to_string(&now()).expect("must serialise");
        let text = rendered.trim_matches('"');
        assert!(text.ends_with('Z'), "{rendered} is not UTC-with-Z");
        assert!(!text.contains('+'), "{rendered} carries a numeric offset");
    }

    #[test]
    fn a_timestamp_round_trips_through_its_wire_form() {
        let instant: Timestamp = "2026-08-20T21:04:05Z".parse().expect("must parse");
        let rendered = serde_json::to_string(&instant).expect("must serialise");
        assert_eq!(rendered, "\"2026-08-20T21:04:05Z\"");
        let back: Timestamp = serde_json::from_str(&rendered).expect("must deserialise");
        assert_eq!(back, instant);
    }
}
