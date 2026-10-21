//! Reading an event write.
//!
//! The schema checks a non-blank name and an end not before the start
//! (`event_ends_after_it_starts`); both are restated here so that a bad value
//! comes back as a 400 naming the field rather than as a 500 carrying a
//! constraint name. The earliest instant a `timestamptz` holds is the bound of
//! `crate::drink::validation`, shared rather than copied.

use uuid::Uuid;

use crate::drink::validation::earliest_ingested_at;
use crate::error::{ApiError, FieldError};
use crate::timestamp::Timestamp;

use super::model::EventRequest;

/// The members of an event that satisfied every rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidEvent {
    /// Name, trimmed, never blank.
    pub name: String,
    /// Instant the event starts.
    pub starts_at: Timestamp,
    /// Instant the event ends, never before it starts; `None` while open-ended.
    pub ends_at: Option<Timestamp>,
}

/// A creation that satisfied every rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidCreation {
    /// The event itself.
    pub event: ValidEvent,
    /// Profiles taking part from the start, each once, in the order first
    /// named. Their existence is the store's to check.
    pub participant_ids: Vec<Uuid>,
}

impl EventRequest {
    /// Reads a creation, reporting every offending member at once.
    ///
    /// # Errors
    ///
    /// Returns a validation failure listing each member that cannot be used.
    pub fn validate_creation(self) -> Result<ValidCreation, ApiError> {
        let mut errors = Vec::new();
        let mut participant_ids: Vec<Uuid> = Vec::new();
        for id in self.participant_ids.iter().flatten().flatten() {
            if !participant_ids.contains(id) {
                participant_ids.push(*id);
            }
        }
        let event = self.read(&mut errors);
        match event {
            Some(event) if errors.is_empty() => Ok(ValidCreation {
                event,
                participant_ids,
            }),
            _ => Err(ApiError::validation(errors)),
        }
    }

    /// Reads a replacement, reporting every offending member at once.
    ///
    /// # Errors
    ///
    /// Returns a validation failure listing each member that cannot be used,
    /// `participant_ids` included whenever it is present.
    pub fn validate_replacement(self) -> Result<ValidEvent, ApiError> {
        let mut errors = Vec::new();
        let names_participants = self.participant_ids.is_some();
        let event = self.read(&mut errors);
        if names_participants {
            errors.push(FieldError::new(
                "participant_ids",
                "is not accepted on a replacement: participants are added and removed \
                 under /events/{id}/participants/{profile_id}",
            ));
        }
        match event {
            Some(event) if errors.is_empty() => Ok(event),
            _ => Err(ApiError::validation(errors)),
        }
    }

    /// Reads the three members of the event itself.
    fn read(self, errors: &mut Vec<FieldError>) -> Option<ValidEvent> {
        let name = match self.name {
            None => {
                errors.push(FieldError::new("name", "is required"));
                None
            }
            Some(name) if name.contains('\0') => {
                errors.push(FieldError::new("name", "must not contain a NUL character"));
                None
            }
            Some(name) if name.trim().is_empty() => {
                errors.push(FieldError::new("name", "must not be blank"));
                None
            }
            Some(name) => Some(name.trim().to_owned()),
        };

        let earliest = earliest_ingested_at();
        let too_early = format!("must not be earlier than {earliest}");
        let starts_at = match self.starts_at {
            None => {
                errors.push(FieldError::new("starts_at", "is required"));
                None
            }
            Some(at) if at < earliest => {
                errors.push(FieldError::new("starts_at", too_early.clone()));
                None
            }
            Some(at) => Some(at),
        };

        let ends_at = match (self.ends_at, starts_at) {
            (Some(end), _) if end < earliest => {
                errors.push(FieldError::new("ends_at", too_early));
                None
            }
            (Some(end), Some(start)) if end < start => {
                errors.push(FieldError::new(
                    "ends_at",
                    "must not be earlier than starts_at",
                ));
                None
            }
            (end, _) => end,
        };

        Some(ValidEvent {
            name: name?,
            starts_at: starts_at?,
            ends_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().expect("a real instant")
    }

    fn sound() -> EventRequest {
        EventRequest {
            name: Some("  Soirée chez Alice  ".to_owned()),
            starts_at: Some(at("2026-10-17T20:00:00Z")),
            ends_at: Some(at("2026-10-18T03:00:00Z")),
            participant_ids: None,
        }
    }

    /// The fields a refused request names, in the order reported.
    fn fields_of<T: std::fmt::Debug>(result: Result<T, ApiError>) -> Vec<String> {
        match result {
            Err(ApiError::Validation(errors)) => errors.into_iter().map(|e| e.field).collect(),
            other => panic!("expected a validation failure, got {other:?}"),
        }
    }

    #[test]
    fn a_sound_creation_is_read_with_its_name_trimmed() {
        let valid = sound().validate_creation().expect("a sound creation");
        assert_eq!(
            valid.event,
            ValidEvent {
                name: "Soirée chez Alice".to_owned(),
                starts_at: at("2026-10-17T20:00:00Z"),
                ends_at: Some(at("2026-10-18T03:00:00Z")),
            }
        );
        assert!(valid.participant_ids.is_empty());
    }

    #[test]
    fn an_event_without_an_end_is_open_ended() {
        let request = EventRequest {
            ends_at: None,
            ..sound()
        };
        assert_eq!(
            request
                .clone()
                .validate_creation()
                .expect("valid")
                .event
                .ends_at,
            None
        );
        assert_eq!(request.validate_replacement().expect("valid").ends_at, None);
    }

    #[test]
    fn a_missing_name_and_start_are_both_reported() {
        let request = EventRequest {
            name: None,
            starts_at: None,
            ..sound()
        };
        assert_eq!(
            fields_of(request.clone().validate_creation()),
            ["name", "starts_at"]
        );
        assert_eq!(
            fields_of(request.validate_replacement()),
            ["name", "starts_at"]
        );
    }

    #[test]
    fn a_blank_name_or_one_carrying_a_nul_is_refused() {
        for name in ["   ", "Soi\0rée"] {
            let request = EventRequest {
                name: Some(name.to_owned()),
                ..sound()
            };
            assert_eq!(fields_of(request.validate_creation()), ["name"], "{name:?}");
        }
    }

    #[test]
    fn an_event_may_end_when_it_starts_but_not_before() {
        let start = at("2026-10-17T20:00:00Z");
        let request = EventRequest {
            ends_at: Some(start),
            ..sound()
        };
        assert_eq!(
            request.validate_creation().expect("valid").event.ends_at,
            Some(start)
        );

        let request = EventRequest {
            ends_at: Some(start - chrono::Duration::microseconds(1)),
            ..sound()
        };
        assert_eq!(fields_of(request.clone().validate_creation()), ["ends_at"]);
        assert_eq!(fields_of(request.validate_replacement()), ["ends_at"]);
    }

    #[test]
    fn an_instant_before_the_earliest_timestamptz_is_refused() {
        let earliest = earliest_ingested_at();
        let request = EventRequest {
            starts_at: Some(earliest),
            ends_at: Some(earliest),
            ..sound()
        };
        let valid = request.validate_creation().expect("the earliest instant");
        assert_eq!(valid.event.starts_at, earliest);

        let too_early = earliest - chrono::Duration::microseconds(1);
        let request = EventRequest {
            starts_at: Some(too_early),
            ends_at: None,
            ..sound()
        };
        assert_eq!(fields_of(request.validate_creation()), ["starts_at"]);

        // Without a usable start, the end is still held to the bound.
        let request = EventRequest {
            starts_at: None,
            ends_at: Some(too_early),
            ..sound()
        };
        assert_eq!(
            fields_of(request.validate_creation()),
            ["starts_at", "ends_at"]
        );
    }

    #[test]
    fn a_profile_named_twice_takes_part_once_in_the_order_first_named() {
        let (a, b, c) = (crate::new_id(), crate::new_id(), crate::new_id());
        let request = EventRequest {
            participant_ids: Some(Some(vec![b, a, b, c, a])),
            ..sound()
        };
        assert_eq!(
            request.validate_creation().expect("valid").participant_ids,
            [b, a, c]
        );
    }

    #[test]
    fn a_replacement_may_not_name_participants() {
        for participant_ids in [vec![], vec![crate::new_id()]] {
            let request = EventRequest {
                participant_ids: Some(Some(participant_ids)),
                ..sound()
            };
            assert_eq!(
                fields_of(request.validate_replacement()),
                ["participant_ids"]
            );
        }
    }

    #[test]
    fn a_replacement_refuses_participant_ids_even_when_null() {
        // Read off the wire, as a client sends it: an explicit `null` is a
        // member present in the payload, and is refused like a list.
        let request: EventRequest = serde_json::from_str(
            r#"{"name":"Soirée","starts_at":"2026-10-17T20:00:00Z","participant_ids":null}"#,
        )
        .expect("must parse");
        assert_eq!(
            fields_of(request.validate_replacement()),
            ["participant_ids"]
        );

        // Absent, it is no member at all; on a creation, `null` names nobody.
        let absent: EventRequest =
            serde_json::from_str(r#"{"name":"Soirée","starts_at":"2026-10-17T20:00:00Z"}"#)
                .expect("must parse");
        assert!(absent.validate_replacement().is_ok());
        let creation: EventRequest = serde_json::from_str(
            r#"{"name":"Soirée","starts_at":"2026-10-17T20:00:00Z","participant_ids":null}"#,
        )
        .expect("must parse");
        assert!(
            creation
                .validate_creation()
                .expect("valid")
                .participant_ids
                .is_empty()
        );
    }
}
