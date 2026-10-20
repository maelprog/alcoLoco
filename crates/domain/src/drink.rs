//! A drink and its components (SPEC.md §4, §5.3, §6.1, §10.0-A, §10.0-B).
//!
//! The same structure carries the three entry modes of SPEC.md §5.3: a free
//! entry (mode 1) is a drink with a single component, a mixology entry (mode 2)
//! one with several. What lives here is everything about a drink that can be
//! decided without I/O: whether it is valid, how much pure alcohol it holds and
//! which name to propose for it. The database checks the structural part of the
//! same rules again (migration `20261021090000_drink_components_invariants`),
//! so that no writer can bypass them.
//!
//! Volumes are millilitres, as everywhere in this crate and in the schema. The
//! cL / % choice of the user decides how a component quantity is *expressed*:
//! with [`QuantityUnit::Cl`] it is a volume (in millilitres, the unit the cL
//! entry is converted to), with [`QuantityUnit::Percent`] a share of the total
//! volume of the drink. This is exactly the meaning of
//! `drink_component.quantity`.

use std::fmt;

use crate::ingested_alcohol_grams;

/// Separator between the two component names of a proposed drink name
/// (SPEC.md §5.3: « composante A – composante B »). An en dash, surrounded by
/// spaces.
pub const NAME_SEPARATOR: &str = " – ";

/// How far, in percentage points, the components of a drink in percent may
/// exceed 100 % in total and still be accepted.
///
/// Decimal percentages are not exact in `f64`: `0.2 + 83.9 + 15.9` evaluates
/// to `100.00000000000001`. Refusing that would refuse a drink the user wrote
/// as exactly 100 %. The same tolerance is written in the database check of
/// migration `20261021090000_drink_components_invariants`, which sums in an
/// order of its own.
pub const PERCENT_SUM_TOLERANCE: f64 = 1e-9;

/// Unit in which the quantities of every component of one drink are expressed
/// (SPEC.md §5.3). Chosen per drink, so every component of a drink shares it.
///
/// Mirrors the `quantity_unit` enum of the database schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuantityUnit {
    /// Each quantity is a volume, in millilitres (entered in cL).
    Cl,
    /// Each quantity is a percentage of the total volume of the drink.
    Percent,
}

/// A component as entered, not yet validated.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentDraft {
    /// Name of the component, e.g. `Vodka` or `Jus d'orange`.
    pub name: String,
    /// Alcohol by volume, in percent, between 0 and 100 inclusive.
    pub abv_percent: f64,
    /// Millilitres when the drink is in [`QuantityUnit::Cl`], percent of the
    /// total volume when it is in [`QuantityUnit::Percent`]. Strictly positive.
    pub quantity: f64,
}

/// A drink as entered, not yet validated.
///
/// The ingestion time, the profile and the optional event are not here: they
/// are references and an instant, checked where they are resolved, not rules
/// of the drink itself.
#[derive(Debug, Clone, PartialEq)]
pub struct DrinkDraft {
    /// Name of the drink, mandatory (SPEC.md §5.3). [`proposed_name`] offers
    /// one; the user may change it.
    pub name: String,
    /// Unit shared by the quantities of every component.
    pub unit: QuantityUnit,
    /// Total volume of the drink, in millilitres. **Mandatory** in
    /// [`QuantityUnit::Percent`] (SPEC.md §10.0-A), **absent** in
    /// [`QuantityUnit::Cl`], where it is derived from the components: never
    /// entered twice, never stored in contradiction with them.
    pub volume_total_ml: Option<f64>,
    /// Ingestion duration, in seconds; zero for a drink downed at once.
    pub ingestion_duration_seconds: i64,
    /// Absorption duration, in seconds; strictly positive.
    pub absorption_duration_seconds: i64,
    /// One or more components, in the order they were added.
    pub components: Vec<ComponentDraft>,
}

/// A drink that satisfies every rule of [`DrinkDraft::validate`].
///
/// Its fields are private so that one can only be obtained through that
/// validation.
#[derive(Debug, Clone, PartialEq)]
pub struct Drink {
    draft: DrinkDraft,
}

/// Why a component is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentRule {
    /// The name is empty or blank.
    BlankName,
    /// The alcohol by volume is outside `[0, 100]`, or not a number.
    AbvOutOfRange,
    /// The quantity is zero, negative, infinite or not a number.
    NonPositiveQuantity,
}

/// One broken rule of a drink.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DrinkError {
    /// The name of the drink is empty or blank.
    BlankName,
    /// The drink has no component.
    NoComponent,
    /// The drink is in percent and carries no total volume (SPEC.md §10.0-A).
    MissingVolumeTotal,
    /// The drink is in cL and carries a total volume, which is derived there.
    VolumeTotalNotAllowed,
    /// The total volume is zero, negative, infinite or not a number.
    NonPositiveVolumeTotal,
    /// The components of a drink in percent sum to more than 100 %.
    PercentSumAbove100 {
        /// The sum found, in percent.
        sum: f64,
    },
    /// The ingestion duration is negative.
    NegativeIngestionDuration,
    /// The absorption duration is zero or negative.
    NonPositiveAbsorptionDuration,
    /// A component breaks one of its own rules.
    Component {
        /// Position of the component, from 0, in the order of addition.
        index: usize,
        /// The rule it breaks.
        rule: ComponentRule,
    },
}

impl DrinkError {
    /// Name of the offending field, in the `snake_case` of the API, with the
    /// index of the component when one is at fault: `components[2].quantity`.
    #[must_use]
    pub fn field(&self) -> String {
        match self {
            Self::BlankName => "name".to_owned(),
            Self::NoComponent | Self::PercentSumAbove100 { .. } => "components".to_owned(),
            Self::MissingVolumeTotal
            | Self::VolumeTotalNotAllowed
            | Self::NonPositiveVolumeTotal => "volume_total_ml".to_owned(),
            Self::NegativeIngestionDuration => "ingestion_duration_seconds".to_owned(),
            Self::NonPositiveAbsorptionDuration => "absorption_duration_seconds".to_owned(),
            Self::Component { index, rule } => {
                let field = match rule {
                    ComponentRule::BlankName => "name",
                    ComponentRule::AbvOutOfRange => "abv_percent",
                    ComponentRule::NonPositiveQuantity => "quantity",
                };
                format!("components[{index}].{field}")
            }
        }
    }
}

impl fmt::Display for DrinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BlankName => f.write_str("a drink needs a name"),
            Self::NoComponent => f.write_str("a drink needs at least one component"),
            Self::MissingVolumeTotal => f.write_str(
                "a drink measured in percent needs its total volume: without it, its alcohol cannot be computed",
            ),
            Self::VolumeTotalNotAllowed => f.write_str(
                "a drink measured in cL takes no total volume: it is the sum of its components",
            ),
            Self::NonPositiveVolumeTotal => {
                f.write_str("the total volume of a drink must be a finite number greater than zero")
            }
            Self::PercentSumAbove100 { sum } => write!(
                f,
                "the components of a drink measured in percent may not exceed 100 % in total, found {sum} %"
            ),
            Self::NegativeIngestionDuration => {
                f.write_str("the ingestion duration may not be negative")
            }
            Self::NonPositiveAbsorptionDuration => {
                f.write_str("the absorption duration must be greater than zero")
            }
            Self::Component { index, rule } => {
                let rule = match rule {
                    ComponentRule::BlankName => "needs a name",
                    ComponentRule::AbvOutOfRange => {
                        "needs an alcohol by volume between 0 and 100 %"
                    }
                    ComponentRule::NonPositiveQuantity => {
                        "needs a quantity that is a finite number greater than zero"
                    }
                };
                write!(f, "component {index} {rule}")
            }
        }
    }
}

impl std::error::Error for DrinkError {}

impl DrinkDraft {
    /// Checks every rule of a drink and reports **all** the broken ones, in a
    /// stable order: the drink's own rules first, then each component's, in
    /// the order of addition.
    ///
    /// # Errors
    ///
    /// The list of broken rules, never empty, when the drink is invalid.
    pub fn validate(self) -> Result<Drink, Vec<DrinkError>> {
        let mut errors = Vec::new();

        if self.name.trim().is_empty() {
            errors.push(DrinkError::BlankName);
        }
        if self.components.is_empty() {
            errors.push(DrinkError::NoComponent);
        }
        match (self.unit, self.volume_total_ml) {
            (QuantityUnit::Percent, None) => errors.push(DrinkError::MissingVolumeTotal),
            (QuantityUnit::Cl, Some(_)) => errors.push(DrinkError::VolumeTotalNotAllowed),
            (QuantityUnit::Percent, Some(volume)) if !is_positive_and_finite(volume) => {
                errors.push(DrinkError::NonPositiveVolumeTotal);
            }
            _ => {}
        }
        if self.ingestion_duration_seconds < 0 {
            errors.push(DrinkError::NegativeIngestionDuration);
        }
        if self.absorption_duration_seconds <= 0 {
            errors.push(DrinkError::NonPositiveAbsorptionDuration);
        }

        let mut quantities_valid = true;
        for (index, component) in self.components.iter().enumerate() {
            if component.name.trim().is_empty() {
                errors.push(DrinkError::Component {
                    index,
                    rule: ComponentRule::BlankName,
                });
            }
            if !(0.0..=100.0).contains(&component.abv_percent) {
                errors.push(DrinkError::Component {
                    index,
                    rule: ComponentRule::AbvOutOfRange,
                });
            }
            if !is_positive_and_finite(component.quantity) {
                quantities_valid = false;
                errors.push(DrinkError::Component {
                    index,
                    rule: ComponentRule::NonPositiveQuantity,
                });
            }
        }

        // Only once every quantity is a number: an invalid one already
        // explains a wrong total, and is reported as such above.
        if self.unit == QuantityUnit::Percent && quantities_valid {
            let sum: f64 = self.components.iter().map(|c| c.quantity).sum();
            if sum > 100.0 + PERCENT_SUM_TOLERANCE {
                errors.push(DrinkError::PercentSumAbove100 { sum });
            }
        }

        if errors.is_empty() {
            Ok(Drink { draft: self })
        } else {
            Err(errors)
        }
    }
}

impl Drink {
    /// Name of the drink, as entered.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.draft.name
    }

    /// Unit shared by the quantities of every component.
    #[must_use]
    pub fn unit(&self) -> QuantityUnit {
        self.draft.unit
    }

    /// Components, in the order they were added.
    #[must_use]
    pub fn components(&self) -> &[ComponentDraft] {
        &self.draft.components
    }

    /// Ingestion duration, in seconds.
    #[must_use]
    pub fn ingestion_duration_seconds(&self) -> i64 {
        self.draft.ingestion_duration_seconds
    }

    /// Absorption duration, in seconds.
    #[must_use]
    pub fn absorption_duration_seconds(&self) -> i64 {
        self.draft.absorption_duration_seconds
    }

    /// Total volume entered, as it is stored: `Some` in percent, `None` in cL.
    #[must_use]
    pub fn entered_volume_total_ml(&self) -> Option<f64> {
        self.draft.volume_total_ml
    }

    /// Total volume of the drink, in millilitres: the one entered in percent,
    /// the sum of the components in cL.
    #[must_use]
    pub fn volume_total_ml(&self) -> f64 {
        match (self.draft.unit, self.draft.volume_total_ml) {
            (QuantityUnit::Percent, Some(volume)) => volume,
            _ => self.draft.components.iter().map(|c| c.quantity).sum(),
        }
    }

    /// Volume of each component, in millilitres, in the order of addition. A
    /// component in percent is converted through the total volume of the drink
    /// (SPEC.md §6.1).
    #[must_use]
    pub fn component_volumes_ml(&self) -> Vec<f64> {
        let total = self.volume_total_ml();
        self.draft
            .components
            .iter()
            .map(|c| match self.draft.unit {
                QuantityUnit::Cl => c.quantity,
                QuantityUnit::Percent => c.quantity / 100.0 * total,
            })
            .collect()
    }

    /// Mass of pure alcohol in the drink, in grams: the sum of the
    /// contributions of its components (SPEC.md §6.1), whatever the unit.
    #[must_use]
    pub fn pure_alcohol_grams(&self) -> f64 {
        self.component_volumes_ml()
            .into_iter()
            .zip(&self.draft.components)
            .map(|(volume_ml, c)| ingested_alcohol_grams(volume_ml, c.abv_percent))
            .sum()
    }
}

/// Name proposed for a drink made of `components`, given in the order they
/// were added (SPEC.md §5.3, §10.0-B):
///
/// * no component → `None`;
/// * one component → its name;
/// * two components → `A – B`, in the order of addition;
/// * three or more → the two components with the **largest quantity**, alcohols
///   and soft drinks alike, joined the same way. A tie is won by the component
///   added first. The two names keep their order of addition, so that removing
///   a minor third component never swaps them.
///
/// Comparing quantities compares volumes, since every component of a drink
/// shares one unit and a percentage is proportional to a volume. The list does
/// not have to be valid — the entry form calls this at every change — and an
/// invalid quantity is ranked by [`f64::total_cmp`], deterministically. Names
/// are trimmed.
#[must_use]
pub fn proposed_name(components: &[ComponentDraft]) -> Option<String> {
    match components {
        [] => None,
        [only] => Some(only.name.trim().to_owned()),
        _ => {
            let largest = largest_quantity_except(components, None);
            let runner_up = largest_quantity_except(components, Some(largest));
            let (first, second) = (largest.min(runner_up), largest.max(runner_up));
            Some(format!(
                "{}{NAME_SEPARATOR}{}",
                components[first].name.trim(),
                components[second].name.trim()
            ))
        }
    }
}

/// Index of the component with the largest quantity, `skip` left out; the
/// first one added wins a tie, since only a strictly larger quantity replaces
/// the current pick. `components` holds at least two entries.
fn largest_quantity_except(components: &[ComponentDraft], skip: Option<usize>) -> usize {
    let mut best: Option<usize> = None;
    for (index, component) in components.iter().enumerate() {
        if Some(index) == skip {
            continue;
        }
        match best {
            Some(current)
                if component
                    .quantity
                    .total_cmp(&components[current].quantity)
                    .is_le() => {}
            _ => best = Some(index),
        }
    }
    best.expect("at least two components")
}

fn is_positive_and_finite(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ETHANOL_DENSITY_G_PER_ML;

    /// Rounding noise allowed between two orders of evaluation of the same sum
    /// of products (values of order 1e1).
    const DERIVED: f64 = 1e-12;

    fn component(name: &str, abv_percent: f64, quantity: f64) -> ComponentDraft {
        ComponentDraft {
            name: name.to_owned(),
            abv_percent,
            quantity,
        }
    }

    /// The cocktail of the acceptance criteria of #13, in cL: 20 cL orange
    /// juice, 4 cL vodka, 2 cL syrup.
    fn screwdriver_cl() -> DrinkDraft {
        DrinkDraft {
            name: "Screwdriver".to_owned(),
            unit: QuantityUnit::Cl,
            volume_total_ml: None,
            ingestion_duration_seconds: 1200,
            absorption_duration_seconds: 1800,
            components: vec![
                component("Jus d'orange", 0.0, 200.0),
                component("Vodka", 40.0, 40.0),
                component("Sirop", 0.0, 20.0),
            ],
        }
    }

    /// A gin and orange of 200 mL, in percent: 25 % gin, 75 % orange juice.
    fn gin_orange_percent() -> DrinkDraft {
        DrinkDraft {
            name: "Gin – Jus d'orange".to_owned(),
            unit: QuantityUnit::Percent,
            volume_total_ml: Some(200.0),
            ingestion_duration_seconds: 0,
            absorption_duration_seconds: 1800,
            components: vec![
                component("Gin", 40.0, 25.0),
                component("Jus d'orange", 0.0, 75.0),
            ],
        }
    }

    fn errors_of(draft: DrinkDraft) -> Vec<DrinkError> {
        draft.validate().expect_err("the draft must be refused")
    }

    // ----- Valid drinks, both units ----------------------------------------

    #[test]
    fn a_drink_in_cl_is_valid_and_derives_its_total_volume() {
        let drink = screwdriver_cl().validate().expect("valid");
        assert_eq!(drink.unit(), QuantityUnit::Cl);
        assert_eq!(drink.entered_volume_total_ml(), None);
        assert!((drink.volume_total_ml() - 260.0).abs() < DERIVED);
        assert_eq!(drink.component_volumes_ml(), vec![200.0, 40.0, 20.0]);
    }

    #[test]
    fn a_drink_in_percent_is_valid_and_converts_its_components_to_volumes() {
        let drink = gin_orange_percent().validate().expect("valid");
        assert_eq!(drink.unit(), QuantityUnit::Percent);
        assert_eq!(drink.entered_volume_total_ml(), Some(200.0));
        assert!((drink.volume_total_ml() - 200.0).abs() < DERIVED);
        let volumes = drink.component_volumes_ml();
        assert!((volumes[0] - 50.0).abs() < DERIVED, "{volumes:?}");
        assert!((volumes[1] - 150.0).abs() < DERIVED, "{volumes:?}");
    }

    #[test]
    fn a_free_entry_is_a_drink_with_a_single_component() {
        let drink = DrinkDraft {
            name: "Bière blonde".to_owned(),
            unit: QuantityUnit::Cl,
            volume_total_ml: None,
            ingestion_duration_seconds: 1200,
            absorption_duration_seconds: 1800,
            components: vec![component("Bière blonde", 5.0, 500.0)],
        }
        .validate()
        .expect("valid");
        assert_eq!(drink.components().len(), 1);
        assert_eq!(drink.name(), "Bière blonde");
    }

    #[test]
    fn components_in_percent_may_sum_to_less_than_100() {
        // The remainder is simply not described (ice, water, …): §4 sets an
        // upper bound only.
        let mut draft = gin_orange_percent();
        draft.components[1].quantity = 50.0;
        assert!(draft.validate().is_ok());
    }

    #[test]
    fn components_in_percent_may_sum_to_exactly_100_despite_rounding() {
        // 0.2 + 83.9 + 15.9 is 100.00000000000001 in f64, summed in this order.
        let sum = 0.2_f64 + 83.9 + 15.9;
        assert!(sum > 100.0, "the case must exercise the tolerance: {sum}");
        let mut draft = gin_orange_percent();
        draft.components = vec![
            component("Gin", 40.0, 0.2),
            component("Jus d'orange", 0.0, 83.9),
            component("Sirop", 0.0, 15.9),
        ];
        assert!(draft.validate().is_ok());
    }

    #[test]
    fn degrees_0_and_100_and_an_instant_ingestion_are_accepted() {
        let mut draft = screwdriver_cl();
        draft.components[1].abv_percent = 100.0;
        draft.ingestion_duration_seconds = 0;
        assert!(draft.validate().is_ok());
    }

    // ----- Pure alcohol -----------------------------------------------------

    #[test]
    fn pure_alcohol_in_cl_sums_the_components() {
        let drink = screwdriver_cl().validate().expect("valid");
        // Only the vodka carries alcohol: 40 mL × 40 % × 0.789.
        let expected = 40.0 * 0.40 * ETHANOL_DENSITY_G_PER_ML;
        assert!((drink.pure_alcohol_grams() - expected).abs() < DERIVED);
    }

    #[test]
    fn pure_alcohol_in_percent_goes_through_the_total_volume() {
        let drink = gin_orange_percent().validate().expect("valid");
        // 25 % of 200 mL = 50 mL of gin at 40 %.
        let expected = 50.0 * 0.40 * ETHANOL_DENSITY_G_PER_ML;
        assert!((drink.pure_alcohol_grams() - expected).abs() < DERIVED);
    }

    #[test]
    fn the_same_drink_holds_the_same_alcohol_in_both_units() {
        let in_cl = DrinkDraft {
            name: "Gin – Tonic".to_owned(),
            unit: QuantityUnit::Cl,
            volume_total_ml: None,
            ingestion_duration_seconds: 600,
            absorption_duration_seconds: 1800,
            components: vec![component("Gin", 37.5, 50.0), component("Tonic", 0.0, 150.0)],
        };
        let in_percent = DrinkDraft {
            unit: QuantityUnit::Percent,
            volume_total_ml: Some(200.0),
            components: vec![component("Gin", 37.5, 25.0), component("Tonic", 0.0, 75.0)],
            ..in_cl.clone()
        };
        let cl = in_cl.validate().expect("valid").pure_alcohol_grams();
        let percent = in_percent.validate().expect("valid").pure_alcohol_grams();
        assert!((cl - percent).abs() < DERIVED, "{cl} vs {percent}");
        assert!((cl - 50.0 * 0.375 * ETHANOL_DENSITY_G_PER_ML).abs() < DERIVED);
    }

    #[test]
    fn several_alcoholic_components_add_up() {
        let mut draft = gin_orange_percent();
        draft.components = vec![
            component("Rhum", 40.0, 20.0),
            component("Triple sec", 30.0, 10.0),
            component("Jus de citron", 0.0, 70.0),
        ];
        let drink = draft.validate().expect("valid");
        // 40 mL at 40 % and 20 mL at 30 %, of 200 mL.
        let expected = (40.0 * 0.40 + 20.0 * 0.30) * ETHANOL_DENSITY_G_PER_ML;
        assert!((drink.pure_alcohol_grams() - expected).abs() < DERIVED);
    }

    // ----- Invalid drinks ---------------------------------------------------

    #[test]
    fn a_drink_in_percent_without_total_volume_is_refused_explicitly() {
        let mut draft = gin_orange_percent();
        draft.volume_total_ml = None;
        let errors = errors_of(draft);
        assert_eq!(errors, vec![DrinkError::MissingVolumeTotal]);
        assert_eq!(errors[0].field(), "volume_total_ml");
        assert_eq!(
            errors[0].to_string(),
            "a drink measured in percent needs its total volume: without it, its alcohol cannot be computed"
        );
    }

    #[test]
    fn a_drink_in_cl_with_a_total_volume_is_refused() {
        let mut draft = screwdriver_cl();
        draft.volume_total_ml = Some(260.0);
        let errors = errors_of(draft);
        assert_eq!(errors, vec![DrinkError::VolumeTotalNotAllowed]);
        assert_eq!(errors[0].field(), "volume_total_ml");
    }

    #[test]
    fn a_non_positive_or_non_finite_total_volume_is_refused() {
        for volume in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut draft = gin_orange_percent();
            draft.volume_total_ml = Some(volume);
            assert_eq!(
                errors_of(draft),
                vec![DrinkError::NonPositiveVolumeTotal],
                "{volume}"
            );
        }
    }

    #[test]
    fn components_in_percent_summing_above_100_are_refused() {
        let mut draft = gin_orange_percent();
        draft.components[1].quantity = 75.5;
        let errors = errors_of(draft);
        assert_eq!(errors.len(), 1, "{errors:?}");
        match errors[0] {
            DrinkError::PercentSumAbove100 { sum } => assert!((sum - 100.5).abs() < DERIVED),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(errors[0].field(), "components");
    }

    #[test]
    fn the_sum_tolerance_does_not_let_a_real_excess_through() {
        let mut draft = gin_orange_percent();
        draft.components[1].quantity = 75.000_001;
        assert!(matches!(
            errors_of(draft)[..],
            [DrinkError::PercentSumAbove100 { .. }]
        ));
    }

    #[test]
    fn quantities_in_cl_have_no_upper_bound_of_100() {
        // 200 + 40 + 20 mL: the bound of 100 is a percentage rule only.
        assert!(screwdriver_cl().validate().is_ok());
    }

    #[test]
    fn a_blank_drink_name_is_refused() {
        for name in ["", "   ", "\t\n"] {
            let mut draft = screwdriver_cl();
            draft.name = name.to_owned();
            let errors = errors_of(draft);
            assert_eq!(errors, vec![DrinkError::BlankName], "{name:?}");
            assert_eq!(errors[0].field(), "name");
        }
    }

    #[test]
    fn a_drink_without_component_is_refused() {
        for unit in [QuantityUnit::Cl, QuantityUnit::Percent] {
            let mut draft = if unit == QuantityUnit::Cl {
                screwdriver_cl()
            } else {
                gin_orange_percent()
            };
            draft.components.clear();
            let errors = errors_of(draft);
            assert_eq!(errors, vec![DrinkError::NoComponent], "{unit:?}");
            assert_eq!(errors[0].field(), "components");
        }
    }

    #[test]
    fn durations_must_be_positive() {
        let mut draft = screwdriver_cl();
        draft.ingestion_duration_seconds = -1;
        draft.absorption_duration_seconds = 0;
        let errors = errors_of(draft);
        assert_eq!(
            errors,
            vec![
                DrinkError::NegativeIngestionDuration,
                DrinkError::NonPositiveAbsorptionDuration
            ]
        );
        assert_eq!(errors[0].field(), "ingestion_duration_seconds");
        assert_eq!(errors[1].field(), "absorption_duration_seconds");

        let mut draft = screwdriver_cl();
        draft.absorption_duration_seconds = -60;
        assert_eq!(
            errors_of(draft),
            vec![DrinkError::NonPositiveAbsorptionDuration]
        );
    }

    #[test]
    fn a_degree_outside_0_to_100_is_refused() {
        for abv in [-0.1, 100.1, f64::NAN, f64::INFINITY] {
            let mut draft = screwdriver_cl();
            draft.components[1].abv_percent = abv;
            let errors = errors_of(draft);
            assert_eq!(
                errors,
                vec![DrinkError::Component {
                    index: 1,
                    rule: ComponentRule::AbvOutOfRange
                }],
                "{abv}"
            );
            assert_eq!(errors[0].field(), "components[1].abv_percent");
        }
    }

    #[test]
    fn a_quantity_that_is_not_strictly_positive_is_refused_in_both_units() {
        for unit in [QuantityUnit::Cl, QuantityUnit::Percent] {
            for quantity in [0.0, -5.0, f64::NAN, f64::INFINITY] {
                let mut draft = if unit == QuantityUnit::Cl {
                    screwdriver_cl()
                } else {
                    gin_orange_percent()
                };
                draft.components[0].quantity = quantity;
                let errors = errors_of(draft);
                assert_eq!(
                    errors,
                    vec![DrinkError::Component {
                        index: 0,
                        rule: ComponentRule::NonPositiveQuantity
                    }],
                    "{unit:?} {quantity}"
                );
                assert_eq!(errors[0].field(), "components[0].quantity");
            }
        }
    }

    #[test]
    fn a_blank_component_name_is_refused() {
        let mut draft = screwdriver_cl();
        draft.components[2].name = "  ".to_owned();
        let errors = errors_of(draft);
        assert_eq!(
            errors,
            vec![DrinkError::Component {
                index: 2,
                rule: ComponentRule::BlankName
            }]
        );
        assert_eq!(errors[0].field(), "components[2].name");
    }

    #[test]
    fn every_broken_rule_is_reported_in_a_stable_order() {
        let draft = DrinkDraft {
            name: String::new(),
            unit: QuantityUnit::Percent,
            volume_total_ml: None,
            ingestion_duration_seconds: -1,
            absorption_duration_seconds: 0,
            components: vec![component("", 120.0, -1.0), component("Tonic", 0.0, 10.0)],
        };
        assert_eq!(
            errors_of(draft),
            vec![
                DrinkError::BlankName,
                DrinkError::MissingVolumeTotal,
                DrinkError::NegativeIngestionDuration,
                DrinkError::NonPositiveAbsorptionDuration,
                DrinkError::Component {
                    index: 0,
                    rule: ComponentRule::BlankName
                },
                DrinkError::Component {
                    index: 0,
                    rule: ComponentRule::AbvOutOfRange
                },
                DrinkError::Component {
                    index: 0,
                    rule: ComponentRule::NonPositiveQuantity
                },
            ]
        );
    }

    #[test]
    fn the_percent_sum_is_not_reported_on_top_of_an_invalid_quantity() {
        // An invalid quantity already explains the sum; reporting both would
        // point the user at a total they cannot fix by itself.
        let mut draft = gin_orange_percent();
        draft.components[0].quantity = f64::INFINITY;
        assert_eq!(
            errors_of(draft),
            vec![DrinkError::Component {
                index: 0,
                rule: ComponentRule::NonPositiveQuantity
            }]
        );
    }

    #[test]
    fn every_error_has_an_english_message() {
        let errors = [
            DrinkError::BlankName,
            DrinkError::NoComponent,
            DrinkError::MissingVolumeTotal,
            DrinkError::VolumeTotalNotAllowed,
            DrinkError::NonPositiveVolumeTotal,
            DrinkError::PercentSumAbove100 { sum: 120.0 },
            DrinkError::NegativeIngestionDuration,
            DrinkError::NonPositiveAbsorptionDuration,
            DrinkError::Component {
                index: 0,
                rule: ComponentRule::BlankName,
            },
            DrinkError::Component {
                index: 0,
                rule: ComponentRule::AbvOutOfRange,
            },
            DrinkError::Component {
                index: 0,
                rule: ComponentRule::NonPositiveQuantity,
            },
        ];
        let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
        for message in &messages {
            assert!(!message.is_empty());
        }
        let mut distinct = messages.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(distinct.len(), messages.len(), "{messages:?}");
        assert!(messages[5].contains("120"), "{}", messages[5]);
    }

    // ----- Proposed name ----------------------------------------------------

    #[test]
    fn no_component_proposes_no_name() {
        assert_eq!(proposed_name(&[]), None);
    }

    #[test]
    fn one_component_proposes_its_name() {
        assert_eq!(
            proposed_name(&[component(" Vodka ", 40.0, 40.0)]).as_deref(),
            Some("Vodka")
        );
    }

    #[test]
    fn two_components_propose_both_in_the_order_of_addition() {
        // The smaller one first: with two components, volume plays no part.
        assert_eq!(
            proposed_name(&[component("Gin", 40.0, 40.0), component("Tonic", 0.0, 160.0)])
                .as_deref(),
            Some("Gin – Tonic")
        );
    }

    #[test]
    fn the_acceptance_cocktail_is_named_after_orange_juice_and_vodka() {
        assert_eq!(
            proposed_name(&screwdriver_cl().components).as_deref(),
            Some("Jus d'orange – Vodka")
        );
    }

    #[test]
    fn a_majority_soft_drink_appears_in_the_name() {
        // SPEC.md §10.0-B: no distinction between alcohols and softs.
        let components = [
            component("Rhum", 40.0, 40.0),
            component("Menthe", 0.0, 5.0),
            component("Eau gazeuse", 0.0, 120.0),
            component("Citron vert", 0.0, 20.0),
        ];
        assert_eq!(
            proposed_name(&components).as_deref(),
            Some("Rhum – Eau gazeuse")
        );
    }

    #[test]
    fn the_two_majority_components_keep_their_order_of_addition() {
        let components = [
            component("Sirop", 0.0, 10.0),
            component("Vodka", 40.0, 40.0),
            component("Jus d'orange", 0.0, 200.0),
        ];
        assert_eq!(
            proposed_name(&components).as_deref(),
            Some("Vodka – Jus d'orange")
        );
    }

    #[test]
    fn a_tie_in_volume_is_won_by_the_component_added_first() {
        let components = [
            component("Sirop", 0.0, 10.0),
            component("Gin", 40.0, 40.0),
            component("Vodka", 40.0, 40.0),
            component("Rhum", 40.0, 40.0),
        ];
        assert_eq!(proposed_name(&components).as_deref(), Some("Gin – Vodka"));

        let all_equal = [
            component("A", 0.0, 10.0),
            component("B", 0.0, 10.0),
            component("C", 0.0, 10.0),
        ];
        assert_eq!(proposed_name(&all_equal).as_deref(), Some("A – B"));
    }

    #[test]
    fn the_name_of_a_drink_in_percent_ranks_by_share() {
        let components = [
            component("Gin", 40.0, 20.0),
            component("Vermouth", 15.0, 30.0),
            component("Campari", 25.0, 50.0),
        ];
        assert_eq!(
            proposed_name(&components).as_deref(),
            Some("Vermouth – Campari")
        );
    }

    #[test]
    fn the_separator_is_a_spaced_en_dash() {
        assert_eq!(NAME_SEPARATOR, " \u{2013} ");
    }
}
