//! Blood alcohol computation engine.
//!
//! This crate is deliberately free of any I/O, database or web framework
//! dependency: it holds pure, deterministic functions so that it can be unit
//! tested in isolation and, later, extracted as a standalone service
//! (see SPEC.md §7 and §9).
//!
//! What lives here today are the reference bricks of SPEC.md §6: the ingested
//! alcohol mass (§6.1), the Watson body water estimate and the resulting `C₀`
//! (§6.2), the elimination rate (§6.3) and the trapezoidal absorption profile
//! (§6.4). Assembling them into a curve — the `AbsorptionProfile` abstraction,
//! the `Σ Rᵢ` superposition of several drinks, the fixed-step forward
//! integration and its floor at zero (§6.5) — is the job of #16.
//!
//! The figures pinned by the tests below are the reference cases of #5: every
//! one of them is derived from the formulas of §6, not copied from a summary.

/// Density of ethanol, in grams per millilitre (SPEC.md §6.1).
pub const ETHANOL_DENSITY_G_PER_ML: f64 = 0.789;

/// Water fraction of blood, used to turn a body water volume into a blood
/// alcohol concentration (SPEC.md §6.2).
pub const BLOOD_WATER_FRACTION: f64 = 0.806;

/// Default zero-order elimination rate, in grams per litre and per hour
/// (SPEC.md §6.3, §10.0-H).
///
/// This is a constant of the `domain` crate, exposed as a function argument so
/// that tests can vary it. It is deliberately neither a profile field nor a
/// database column, and it applies **once** to the body, however many drinks
/// are being absorbed at the same time (SPEC.md §6.5).
pub const ELIMINATION_RATE_G_PER_L_PER_H: f64 = 0.15;

/// Historical Widmark distribution ratio for men, used as a fallback when
/// Watson cannot be applied (SPEC.md §6.2).
pub const WIDMARK_R_MALE: f64 = 0.68;

/// Historical Widmark distribution ratio for women, used as a fallback when
/// Watson cannot be applied (SPEC.md §6.2).
pub const WIDMARK_R_FEMALE: f64 = 0.55;

/// The two sexes the Watson equations distinguish (SPEC.md §6.2).
///
/// The model has no third value: mirrors the `sex` enum of the database schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sex {
    Male,
    Female,
}

/// Mass of pure alcohol, in grams, contained in a given volume of a beverage.
///
/// `volume_ml` is the volume of the beverage in millilitres, `abv_percent` its
/// alcohol by volume in percent. Reference formula of SPEC.md §6.1:
/// `A (g) = volume(mL) × degree(%) / 100 × 0.789`.
#[must_use]
pub fn ingested_alcohol_grams(volume_ml: f64, abv_percent: f64) -> f64 {
    volume_ml * abv_percent / 100.0 * ETHANOL_DENSITY_G_PER_ML
}

/// Total body water, in litres, estimated by the Watson (1980) equations
/// (SPEC.md §6.2).
///
/// ```text
/// TBW_male   (L) = 2.447 − 0.09516 × age + 0.1074 × height(cm) + 0.3362 × weight(kg)
/// TBW_female (L) = −2.097            + 0.1069 × height(cm) + 0.2466 × weight(kg)
/// ```
///
/// The age coefficient is `0.09516`, the value of the source paper (Watson PE,
/// Watson ID, Batt RD, *Am J Clin Nutr* 1980;33(1):27-39). The `0.09156`
/// variant found on consumer calculators has the two middle digits transposed
/// and is **not** the published value.
///
/// The female equation carries no age term: `age_years` is therefore ignored
/// for [`Sex::Female`]. That is a property of the model, not an oversight.
///
/// `age_years` is the age **at the ingestion time** of the drink, derived from
/// the birth date of the settings version in force (SPEC.md §10.0-K); it is
/// taken as a real number so that a fraction of a year can be expressed.
#[must_use]
pub fn watson_total_body_water_litres(
    sex: Sex,
    weight_kg: f64,
    height_cm: f64,
    age_years: f64,
) -> f64 {
    match sex {
        Sex::Male => 2.447 - 0.09516 * age_years + 0.1074 * height_cm + 0.3362 * weight_kg,
        Sex::Female => -2.097 + 0.1069 * height_cm + 0.2466 * weight_kg,
    }
}

/// Total body water, in litres, implied by the historical Widmark ratios
/// (SPEC.md §6.2).
///
/// SPEC.md §6.2 states `r = TBW / (0.806 × M)`, so the body water an `r` stands
/// for is `TBW = r × M × 0.806`. Feeding it to [`initial_bac_g_per_l`] gives
/// back exactly Widmark's `C₀ = A / (r × M)`.
#[must_use]
pub fn widmark_total_body_water_litres(sex: Sex, weight_kg: f64) -> f64 {
    let r = match sex {
        Sex::Male => WIDMARK_R_MALE,
        Sex::Female => WIDMARK_R_FEMALE,
    };
    r * weight_kg * BLOOD_WATER_FRACTION
}

/// Total body water, in litres, for a set of profile parameters.
///
/// Uses Watson when height and age are both available, and falls back on the
/// historical Widmark ratios when either is missing, as SPEC.md §6.2 requires.
/// The fallback condition is stated by the spec on *either* input, so a missing
/// age triggers it for women too, even though the female Watson equation would
/// not have used that age.
#[must_use]
pub fn total_body_water_litres(
    sex: Sex,
    weight_kg: f64,
    height_cm: Option<f64>,
    age_years: Option<f64>,
) -> f64 {
    match (height_cm, age_years) {
        (Some(height_cm), Some(age_years)) => {
            watson_total_body_water_litres(sex, weight_kg, height_cm, age_years)
        }
        _ => widmark_total_body_water_litres(sex, weight_kg),
    }
}

/// Blood alcohol concentration, in grams per litre, produced by `alcohol_grams`
/// spread over `total_body_water_litres` of body water (SPEC.md §6.2).
///
/// `C₀ = 0.806 × A / TBW`. The function is linear in `alcohol_grams`, which is
/// what makes the `C₀` of several simultaneous drinks the sum of theirs.
#[must_use]
pub fn initial_bac_g_per_l(alcohol_grams: f64, total_body_water_litres: f64) -> f64 {
    BLOOD_WATER_FRACTION * alcohol_grams / total_body_water_litres
}

/// Time, in hours, needed to eliminate `bac_g_per_l` at
/// `elimination_rate_g_per_l_per_h` (SPEC.md §6.3).
///
/// Elimination is zero-order, so this is simply `C / β`. It is measured from
/// the moment the concentration is `bac_g_per_l`; for a single drink whose
/// alcohol is fully absorbed before sobriety, the whole tail is `C₀ − β × t`
/// and the drink is back to zero at `C₀ / β` after its ingestion time, whatever
/// the absorption profile.
///
/// The rate is a parameter so that tests can vary it, but the value used by the
/// application is [`ELIMINATION_RATE_G_PER_L_PER_H`], and it is applied **once**
/// to the body, never once per drink.
#[must_use]
pub fn hours_until_sober(bac_g_per_l: f64, elimination_rate_g_per_l_per_h: f64) -> f64 {
    bac_g_per_l / elimination_rate_g_per_l_per_h
}

/// Rate `R(τ)`, in grams per hour, at which the alcohol of one drink appears in
/// the blood — the trapezoidal profile of SPEC.md §6.4.
///
/// With `τ = elapsed_hours`, `a = ingestion_hours`, `b = absorption_hours`,
/// `m = min(a, b)` and `M = max(a, b)`:
///
/// ```text
///          ⎧ A × τ / (a × b)             0 ≤ τ < m      (rise)
/// R(τ)  =  ⎨ A / M                       m ≤ τ < M      (plateau)
///          ⎪ A × (a + b − τ) / (a × b)   M ≤ τ < a + b  (fall)
///          ⎩ 0                           otherwise
/// ```
///
/// Documented degeneracies:
///
/// | Case | Shape |
/// |---|---|
/// | `a = 0` or `b = 0` | rectangle of height `A / max(a, b)` — linear ramp |
/// | `a = 0` **and** `b = 0` | instantaneous ingestion — pure Widmark |
/// | `a = b` | isosceles triangle, no plateau |
///
/// When both durations are zero the rate is a Dirac impulse: the function
/// returns [`f64::INFINITY`] at `τ = 0` and `0` elsewhere. That case is meant to
/// be handled through [`absorbed_alcohol_grams`], which steps cleanly from `0`
/// to `A`; returning `0` here instead would silently lose the whole dose.
///
/// This profile is the **default implementation**, not a settled product
/// decision: it is to be confronted with the other candidates before the V1
/// release (SPEC.md §10.1, issue #37).
#[must_use]
pub fn absorption_rate_g_per_h(
    alcohol_grams: f64,
    ingestion_hours: f64,
    absorption_hours: f64,
    elapsed_hours: f64,
) -> f64 {
    let (a, b, tau) = (ingestion_hours, absorption_hours, elapsed_hours);
    if a == 0.0 && b == 0.0 {
        // Dirac impulse: the whole dose appears at once. See the note above.
        return if tau == 0.0 { f64::INFINITY } else { 0.0 };
    }
    let (shortest, longest) = (a.min(b), a.max(b));
    if tau < 0.0 || tau >= a + b {
        // Nothing before the first sip, nothing left after the last fraction.
        0.0
    } else if tau < shortest {
        // Rise. `shortest` is zero when one duration is, so this branch is
        // never taken in that case and `a × b = 0` is never divided by.
        alcohol_grams * tau / (a * b)
    } else if tau < longest {
        alcohol_grams / longest
    } else {
        // Fall. Likewise unreachable when a duration is zero, since `longest`
        // is then `a + b`.
        alcohol_grams * (a + b - tau) / (a * b)
    }
}

/// Alcohol mass, in grams, absorbed since `t₀` — the exact primitive of
/// [`absorption_rate_g_per_h`], i.e. `∫₀^τ R`.
///
/// It grows from `0` to `alcohol_grams`, reached at `τ = a + b`: the area under
/// `R` is exactly `A`, all of the dose ends up absorbed (SPEC.md §6.4). Being a
/// closed form, it holds for the degenerate cases too, including the Dirac of
/// `a = b = 0`, where it steps from `0` to `A` at `τ = 0`.
///
/// This is the quantity a curve needs: `κ × absorbed(τ)` is the concentration
/// the drink has contributed so far. The fixed-step integration of several
/// drinks and the floor at zero belong to #16.
#[must_use]
pub fn absorbed_alcohol_grams(
    alcohol_grams: f64,
    ingestion_hours: f64,
    absorption_hours: f64,
    elapsed_hours: f64,
) -> f64 {
    let (a, b, tau) = (ingestion_hours, absorption_hours, elapsed_hours);
    if tau < 0.0 {
        return 0.0;
    }
    if a == 0.0 && b == 0.0 {
        // The impulse of the pure Widmark degeneracy is at τ = 0.
        return alcohol_grams;
    }
    let (shortest, longest) = (a.min(b), a.max(b));
    if tau >= a + b {
        alcohol_grams
    } else if tau < shortest {
        // Area of the rising triangle.
        alcohol_grams * tau * tau / (2.0 * a * b)
    } else if tau < longest {
        // Whole rise — `A × m² / (2ab) = A × m / (2M)`, since `ab = m × M` —
        // plus the plateau run so far.
        alcohol_grams * shortest / (2.0 * longest) + alcohol_grams * (tau - shortest) / longest
    } else {
        // Everything but the triangle still to come.
        let left = a + b - tau;
        alcohol_grams - alcohol_grams * left * left / (2.0 * a * b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- The SPEC.md §10.1 comparison case -------------------------------
    //
    // Man, 80 kg, 180 cm, 30 years old; 50 cL at 5 % vol; ingestion 20 min,
    // absorption 30 min, β = 0.15. Every figure asserted below is recomputed
    // from the formulas of §6 rather than copied from the tables of §10.1;
    // where §10.1 quotes a rounded value, the test also checks the agreement.

    const REF_WEIGHT_KG: f64 = 80.0;
    const REF_HEIGHT_CM: f64 = 180.0;
    const REF_AGE_YEARS: f64 = 30.0;
    const REF_VOLUME_ML: f64 = 500.0;
    const REF_ABV_PERCENT: f64 = 5.0;
    const REF_INGESTION_HOURS: f64 = 20.0 / 60.0;
    const REF_ABSORPTION_HOURS: f64 = 30.0 / 60.0;

    /// Tolerance for a value this test file derives itself in `f64`: the
    /// assertion is meant to pin the last bits, only rounding noise between two
    /// orders of evaluation is allowed (the values compared are of order 1e1,
    /// whose ulp is ~1e-15).
    const DERIVED: f64 = 1e-12;

    /// Tolerance for a figure SPEC.md quotes rounded to the third decimal:
    /// half a unit of that decimal. Asserting tighter would test the rounding
    /// of the spec, not the formula.
    const SPEC_THIRD_DECIMAL: f64 = 5e-4;

    /// Tolerance for a figure SPEC.md quotes rounded to the second decimal.
    const SPEC_SECOND_DECIMAL: f64 = 5e-3;

    fn ref_alcohol_grams() -> f64 {
        ingested_alcohol_grams(REF_VOLUME_ML, REF_ABV_PERCENT)
    }

    fn ref_body_water_litres() -> f64 {
        watson_total_body_water_litres(Sex::Male, REF_WEIGHT_KG, REF_HEIGHT_CM, REF_AGE_YEARS)
    }

    /// Concentration of the reference drink alone, `τ` hours after its
    /// ingestion: `κ × absorbed(τ) − β × τ`.
    ///
    /// This composition belongs to the test, not to the crate: it is the
    /// closed-form single-drink curve, valid while the concentration stays
    /// positive. The superposition of several drinks, the forward integration
    /// and the floor at zero are #16's.
    fn ref_bac_at(ingestion_hours: f64, absorption_hours: f64, elapsed_hours: f64) -> f64 {
        let absorbed = absorbed_alcohol_grams(
            ref_alcohol_grams(),
            ingestion_hours,
            absorption_hours,
            elapsed_hours,
        );
        initial_bac_g_per_l(absorbed, ref_body_water_litres())
            - ELIMINATION_RATE_G_PER_L_PER_H * elapsed_hours
    }

    // ----- §6.1 — ingested alcohol -----------------------------------------

    #[test]
    fn a_25_cl_beer_at_5_percent_holds_about_10_grams_of_alcohol() {
        let grams = ingested_alcohol_grams(250.0, 5.0);
        assert!(
            (grams - 9.8625).abs() < 1e-9,
            "unexpected alcohol mass: {grams}"
        );
    }

    #[test]
    fn a_soft_drink_holds_no_alcohol() {
        assert_eq!(ingested_alcohol_grams(330.0, 0.0), 0.0);
    }

    #[test]
    fn the_reference_half_litre_at_5_percent_holds_19_725_grams() {
        // 500 × 5 / 100 × 0.789 = 19.725 g, the figure §10.1 states.
        let grams = ref_alcohol_grams();
        assert!(
            (grams - 19.725).abs() < DERIVED,
            "unexpected alcohol mass: {grams}"
        );
    }

    #[test]
    fn a_composed_drink_sums_the_alcohol_of_its_components() {
        // 25 cL of a cocktail: one component given as 20 % of the total volume
        // at 40 % vol, one given as 10 cL at 12 % vol. A component expressed in
        // percent is first turned into a volume through the total volume of the
        // drink, which §10.0-A makes mandatory in that case.
        let total_volume_ml = 250.0;
        let spirit_ml = total_volume_ml * 20.0 / 100.0; // 50 mL
        let spirit = ingested_alcohol_grams(spirit_ml, 40.0); // 15.78 g
        let wine = ingested_alcohol_grams(100.0, 12.0); // 9.468 g

        assert!((spirit - 15.78).abs() < DERIVED, "spirit: {spirit}");
        assert!((wine - 9.468).abs() < DERIVED, "wine: {wine}");
        assert!(
            (spirit + wine - 25.248).abs() < DERIVED,
            "composed drink: {}",
            spirit + wine
        );
    }

    // ----- §6.2 — Watson body water and C₀ ---------------------------------

    #[test]
    fn watson_body_water_of_the_reference_man() {
        // 2.447 − 0.09516 × 30 + 0.1074 × 180 + 0.3362 × 80
        //   = 2.447 − 2.8548 + 19.332 + 26.896 = 45.8202 L
        let tbw = ref_body_water_litres();
        assert!((tbw - 45.8202).abs() < DERIVED, "unexpected TBW: {tbw}");
        // §10.1 quotes 45.82 L for this template.
        assert!(
            (tbw - 45.82).abs() < SPEC_SECOND_DECIMAL,
            "disagrees with SPEC.md §10.1: {tbw}"
        );
    }

    #[test]
    fn watson_uses_the_published_age_coefficient_not_the_transposed_variant() {
        // One year of age removes exactly the age coefficient from the male
        // equation, which pins it without depending on the other terms.
        let at_30 =
            watson_total_body_water_litres(Sex::Male, REF_WEIGHT_KG, REF_HEIGHT_CM, REF_AGE_YEARS);
        let at_31 = watson_total_body_water_litres(
            Sex::Male,
            REF_WEIGHT_KG,
            REF_HEIGHT_CM,
            REF_AGE_YEARS + 1.0,
        );
        assert!(
            (at_30 - at_31 - 0.09516).abs() < DERIVED,
            "age coefficient is not 0.09516: {}",
            at_30 - at_31
        );
        // The 0.09156 variant of consumer calculators would give 45.9282 L on
        // this template, and 0.346 g/L instead of 0.347 g/L.
        assert!(
            (at_30 - 45.9282).abs() > 0.1,
            "the transposed 0.09156 variant has crept in: {at_30}"
        );
    }

    #[test]
    fn watson_body_water_of_a_woman() {
        // −2.097 + 0.1069 × 165 + 0.2466 × 65
        //   = −2.097 + 17.6385 + 16.029 = 31.5705 L
        let tbw = watson_total_body_water_litres(Sex::Female, 65.0, 165.0, 30.0);
        assert!((tbw - 31.5705).abs() < DERIVED, "unexpected TBW: {tbw}");
    }

    #[test]
    fn the_female_watson_equation_has_no_age_term() {
        let young = watson_total_body_water_litres(Sex::Female, 65.0, 165.0, 20.0);
        let older = watson_total_body_water_litres(Sex::Female, 65.0, 165.0, 60.0);
        assert!(
            (young - older).abs() < DERIVED,
            "age changed the female estimate: {young} vs {older}"
        );
    }

    #[test]
    fn complete_parameters_use_watson() {
        let tbw = total_body_water_litres(
            Sex::Male,
            REF_WEIGHT_KG,
            Some(REF_HEIGHT_CM),
            Some(REF_AGE_YEARS),
        );
        assert!((tbw - 45.8202).abs() < DERIVED, "unexpected TBW: {tbw}");
    }

    #[test]
    fn a_missing_height_falls_back_on_the_widmark_ratios() {
        // TBW = r × M × 0.806, so that C₀ = 0.806 × A / TBW = A / (r × M).
        let man = total_body_water_litres(Sex::Male, 80.0, None, Some(30.0));
        assert!(
            (man - 0.68 * 80.0 * 0.806).abs() < DERIVED,
            "male fallback: {man}"
        );
        let woman = total_body_water_litres(Sex::Female, 60.0, None, Some(30.0));
        assert!(
            (woman - 0.55 * 60.0 * 0.806).abs() < DERIVED,
            "female fallback: {woman}"
        );

        // And the resulting concentration is exactly Widmark's A / (r × M).
        let grams = ref_alcohol_grams();
        let bac = initial_bac_g_per_l(grams, man);
        assert!(
            (bac - grams / (WIDMARK_R_MALE * 80.0)).abs() < DERIVED,
            "fallback C₀ is not Widmark's: {bac}"
        );
    }

    #[test]
    fn a_missing_age_falls_back_on_the_widmark_ratios() {
        // SPEC.md §6.2 states the fallback on either missing input, so it also
        // fires for a woman, whose Watson equation would not have used the age.
        let man = total_body_water_litres(Sex::Male, 80.0, Some(180.0), None);
        assert!(
            (man - WIDMARK_R_MALE * 80.0 * BLOOD_WATER_FRACTION).abs() < DERIVED,
            "male fallback: {man}"
        );
        let woman = total_body_water_litres(Sex::Female, 65.0, Some(165.0), None);
        assert!(
            (woman - WIDMARK_R_FEMALE * 65.0 * BLOOD_WATER_FRACTION).abs() < DERIVED,
            "female fallback: {woman}"
        );
        assert!(
            (woman - 31.5705).abs() > 1.0,
            "the Watson estimate was used despite the missing age: {woman}"
        );
    }

    #[test]
    fn initial_bac_of_the_reference_case() {
        // C₀ = 0.806 × 19.725 / 45.8202 = 0.346 972 514 305 917 5 g/L
        let bac = initial_bac_g_per_l(ref_alcohol_grams(), ref_body_water_litres());
        assert!(
            (bac - 0.346_972_514_305_917_5).abs() < DERIVED,
            "unexpected C₀: {bac}"
        );
        // §10.1 quotes 0.347 g/L for this case.
        assert!(
            (bac - 0.347).abs() < SPEC_THIRD_DECIMAL,
            "disagrees with SPEC.md §10.1: {bac}"
        );
    }

    #[test]
    fn initial_bac_of_a_set_of_templates() {
        // Every expected value below is 0.806 × A / TBW, with A from §6.1 and
        // TBW from the Watson equation of the sex considered.
        let cases = [
            // (sex, weight, height, age, volume mL, abv %, expected C₀)
            (
                Sex::Male,
                80.0,
                180.0,
                30.0,
                500.0,
                5.0,
                0.346_972_514_305_917_5,
            ),
            (
                Sex::Female,
                65.0,
                165.0,
                30.0,
                500.0,
                5.0,
                0.503_582_458_307_597_3,
            ),
            (
                Sex::Male,
                95.0,
                175.0,
                45.0,
                330.0,
                8.0,
                0.343_334_756_681_145_6,
            ),
            (
                Sex::Female,
                55.0,
                160.0,
                25.0,
                120.0,
                12.0,
                0.320_526_762_338_116_94,
            ),
        ];

        for (sex, weight, height, age, volume, abv, expected) in cases {
            let tbw = watson_total_body_water_litres(sex, weight, height, age);
            let bac = initial_bac_g_per_l(ingested_alcohol_grams(volume, abv), tbw);
            assert!(
                (bac - expected).abs() < DERIVED,
                "{sex:?} {weight} kg / {height} cm / {age} y, {volume} mL at {abv} %: \
                 expected {expected}, got {bac}"
            );
        }
    }

    #[test]
    fn initial_bac_is_linear_in_the_ingested_mass() {
        // This is what makes the C₀ of simultaneous drinks the sum of theirs.
        let tbw = ref_body_water_litres();
        let one = initial_bac_g_per_l(10.0, tbw);
        let other = initial_bac_g_per_l(9.725, tbw);
        let together = initial_bac_g_per_l(19.725, tbw);
        assert!(
            (one + other - together).abs() < DERIVED,
            "{one} + {other} != {together}"
        );
    }

    // ----- §6.3 — elimination, tail and return to zero ----------------------

    #[test]
    fn the_reference_drink_is_back_to_zero_after_2_31_hours() {
        // C₀ / β = 0.346 972 514 305 917 5 / 0.15 = 2.313 150 095 372 783 7 h,
        // i.e. 138.789 min after ingestion.
        let bac = initial_bac_g_per_l(ref_alcohol_grams(), ref_body_water_litres());
        let hours = hours_until_sober(bac, ELIMINATION_RATE_G_PER_L_PER_H);
        assert!(
            (hours - 2.313_150_095_372_783_7).abs() < DERIVED,
            "unexpected sobering time: {hours} h"
        );
        assert!(
            (hours * 60.0 - 138.789_005_722_367).abs() < 1e-9,
            "unexpected sobering time: {} min",
            hours * 60.0
        );
    }

    #[test]
    fn the_elimination_rate_is_a_parameter() {
        // §10.0-H: β is a constant of the crate, exposed as an argument so that
        // tests can vary it — never a profile field.
        let slow = hours_until_sober(0.6, 0.10);
        let fast = hours_until_sober(0.6, 0.20);
        assert!((slow - 6.0).abs() < DERIVED, "at 0.10 g/L/h: {slow} h");
        assert!((fast - 3.0).abs() < DERIVED, "at 0.20 g/L/h: {fast} h");
    }

    #[test]
    fn all_the_alcohol_is_absorbed_by_the_end_of_the_profile() {
        // The area under R is exactly A, for every shape the trapezoid takes.
        let grams = ref_alcohol_grams();
        for (a, b) in [
            (REF_INGESTION_HOURS, REF_ABSORPTION_HOURS), // trapezoid
            (0.0, REF_INGESTION_HOURS + REF_ABSORPTION_HOURS), // linear ramp
            (REF_INGESTION_HOURS + REF_ABSORPTION_HOURS, 0.0), // ramp, other way
            (0.0, 0.0),                                  // pure Widmark
            (0.25, 0.25),                                // isosceles triangle
        ] {
            let absorbed = absorbed_alcohol_grams(grams, a, b, a + b);
            assert!(
                (absorbed - grams).abs() < DERIVED,
                "a = {a} h, b = {b} h: absorbed {absorbed} of {grams} g"
            );
        }
    }

    #[test]
    fn after_absorption_every_profile_falls_back_on_the_same_line() {
        // §10.1: the variants converge after the peak, on C₀ − β × t. One hour
        // after ingestion, all the profiles below are done absorbing, so they
        // must agree to the last bits.
        let expected = initial_bac_g_per_l(ref_alcohol_grams(), ref_body_water_litres())
            - ELIMINATION_RATE_G_PER_L_PER_H;
        for (a, b) in [
            (REF_INGESTION_HOURS, REF_ABSORPTION_HOURS),
            (0.0, REF_INGESTION_HOURS + REF_ABSORPTION_HOURS),
            (0.0, 0.0),
            (0.25, 0.25),
        ] {
            let bac = ref_bac_at(a, b, 1.0);
            assert!(
                (bac - expected).abs() < DERIVED,
                "a = {a} h, b = {b} h: {bac} instead of {expected}"
            );
        }
        assert!(
            (expected - 0.196_972_514_305_917_53).abs() < DERIVED,
            "unexpected tail value: {expected}"
        );
    }

    #[test]
    fn no_elimination_debt_is_carried_over_to_a_later_drink() {
        // §6.3: the concentration is floored at zero, so nothing is owed once
        // sober. The floored integration that shows this on a whole curve is
        // #16's; what the bricks laid here can show is that the sobering time
        // of a drink depends on its own C₀ alone — the idle hours since the
        // previous drink are neither credited nor charged.
        let bac = initial_bac_g_per_l(ref_alcohol_grams(), ref_body_water_litres());
        let alone = hours_until_sober(bac, ELIMINATION_RATE_G_PER_L_PER_H);

        for idle_hours in [0.0, 1.0, 5.0, 48.0] {
            let second_ingested_at = alone + idle_hours;
            let second_sober_at =
                second_ingested_at + hours_until_sober(bac, ELIMINATION_RATE_G_PER_L_PER_H);
            assert!(
                (second_sober_at - second_ingested_at - alone).abs() < DERIVED,
                "{idle_hours} idle hours changed the second drink: \
                 {second_sober_at} − {second_ingested_at} != {alone}"
            );
        }
    }

    #[test]
    fn beta_applies_once_to_the_body_not_once_per_drink() {
        // §6.5: two drinks emptied at the same time double the concentration,
        // hence double the sobering time. Applying β once per drink would give
        // back the sobering time of a single one.
        let tbw = ref_body_water_litres();
        let one = ref_alcohol_grams();
        let single = hours_until_sober(
            initial_bac_g_per_l(one, tbw),
            ELIMINATION_RATE_G_PER_L_PER_H,
        );
        let pair = hours_until_sober(
            initial_bac_g_per_l(one + one, tbw),
            ELIMINATION_RATE_G_PER_L_PER_H,
        );
        assert!(
            (single - 2.313_150_095_372_783_7).abs() < DERIVED,
            "one drink: {single} h"
        );
        assert!(
            (pair - 4.626_300_190_745_567).abs() < DERIVED,
            "two drinks: {pair} h"
        );
        assert!(
            (pair - 2.0 * single).abs() < DERIVED,
            "β was not applied once: {pair} h for two drinks, {single} h for one"
        );
    }

    // ----- §6.4 — the trapezoidal rate itself -------------------------------
    //
    // These assertions describe R, the function §6.4 defines, not the shape of
    // the concentration curve: they stay valid whatever #37 decides to make the
    // default profile.

    #[test]
    fn the_trapezoidal_rate_follows_its_three_branches() {
        // A = 19.725 g, a = 1/3 h, b = 1/2 h, so a × b = 1/6 and A / M = 39.45 g/h.
        let grams = ref_alcohol_grams();
        let r =
            |tau| absorption_rate_g_per_h(grams, REF_INGESTION_HOURS, REF_ABSORPTION_HOURS, tau);
        // The rise is linear: A × τ / (a × b) = 118.35 × τ.
        assert!((r(0.0) - 0.0).abs() < DERIVED, "at τ = 0: {}", r(0.0));
        assert!(
            (r(1.0 / 6.0) - 19.725).abs() < DERIVED,
            "mid-rise: {}",
            r(1.0 / 6.0)
        );
        // Plateau at A / max(a, b), from τ = m to τ = M.
        assert!(
            (r(REF_INGESTION_HOURS) - 39.45).abs() < DERIVED,
            "start of plateau: {}",
            r(REF_INGESTION_HOURS)
        );
        assert!((r(0.4) - 39.45).abs() < DERIVED, "plateau: {}", r(0.4));
        // The fall is the mirror image: A × (a + b − τ) / (a × b).
        assert!((r(0.7) - 15.78).abs() < 1e-9, "on the fall: {}", r(0.7));
        // Outside [0, a + b), nothing appears any more.
        assert!((r(-1.0)).abs() < DERIVED, "before t₀: {}", r(-1.0));
        assert!(
            (r(REF_INGESTION_HOURS + REF_ABSORPTION_HOURS)).abs() < DERIVED,
            "at a + b: {}",
            r(REF_INGESTION_HOURS + REF_ABSORPTION_HOURS)
        );
        assert!((r(0.9)).abs() < DERIVED, "after a + b: {}", r(0.9));
    }

    #[test]
    fn the_area_under_the_rate_is_the_ingested_mass() {
        // Cross-check of the two functions: the closed-form primitive against a
        // trapezium-rule quadrature of the rate. The tolerance is that of the
        // quadrature, not of the formulas: the rule is exact on each linear
        // piece and only errs on the steps of the profile, by O(h²) there.
        let grams = ref_alcohol_grams();
        let steps: u32 = 100_000;
        let upper = 0.6; // inside the fall, so all three branches are covered
        let h = upper / f64::from(steps);
        let mut area = 0.0;
        for i in 0..steps {
            let left = absorption_rate_g_per_h(
                grams,
                REF_INGESTION_HOURS,
                REF_ABSORPTION_HOURS,
                f64::from(i) * h,
            );
            let right = absorption_rate_g_per_h(
                grams,
                REF_INGESTION_HOURS,
                REF_ABSORPTION_HOURS,
                f64::from(i + 1) * h,
            );
            area += 0.5 * (left + right) * h;
        }
        let closed_form =
            absorbed_alcohol_grams(grams, REF_INGESTION_HOURS, REF_ABSORPTION_HOURS, upper);
        assert!(
            (area - closed_form).abs() < 1e-6,
            "quadrature {area} g vs closed form {closed_form} g"
        );
        assert!(
            (closed_form - 16.503_25).abs() < DERIVED,
            "unexpected absorbed mass at τ = 0.6 h: {closed_form}"
        );
    }

    #[test]
    fn a_single_zero_duration_degenerates_into_a_rectangle() {
        // §6.4: a = 0 or b = 0 gives a rectangular rate — a linear ramp of
        // absorption — of height A / max(a, b) over [0, max(a, b)).
        let grams = ref_alcohol_grams();
        for (a, b) in [(0.0, 0.5), (0.5, 0.0)] {
            for tau in [0.0, 0.25, 0.4999] {
                let rate = absorption_rate_g_per_h(grams, a, b, tau);
                assert!(
                    (rate - grams / 0.5).abs() < DERIVED,
                    "a = {a}, b = {b}, τ = {tau}: {rate} g/h"
                );
            }
            assert!(
                absorption_rate_g_per_h(grams, a, b, 0.5).abs() < DERIVED,
                "a = {a}, b = {b}: the rectangle did not close at τ = 0.5 h"
            );
            // Absorption is then linear in time.
            let absorbed = absorbed_alcohol_grams(grams, a, b, 0.25);
            assert!(
                (absorbed - grams / 2.0).abs() < DERIVED,
                "a = {a}, b = {b}: {absorbed} g absorbed at half time"
            );
        }
    }

    #[test]
    fn two_zero_durations_degenerate_into_instantaneous_widmark() {
        // §6.4: a = 0 and b = 0 is an instantaneous ingestion. The rate is a
        // Dirac impulse, so it is the absorbed mass that carries the dose.
        let grams = ref_alcohol_grams();
        assert!(
            absorption_rate_g_per_h(grams, 0.0, 0.0, 0.0).is_infinite(),
            "the impulse at τ = 0 was not reported as such"
        );
        assert!(
            absorption_rate_g_per_h(grams, 0.0, 0.0, 0.1).abs() < DERIVED,
            "the impulse leaked past τ = 0"
        );
        assert!(
            absorbed_alcohol_grams(grams, 0.0, 0.0, -0.1).abs() < DERIVED,
            "alcohol absorbed before ingestion"
        );
        for tau in [0.0, 0.001, 1.0] {
            let absorbed = absorbed_alcohol_grams(grams, 0.0, 0.0, tau);
            assert!(
                (absorbed - grams).abs() < DERIVED,
                "at τ = {tau}: {absorbed} g of {grams} g absorbed"
            );
        }
        // Which is Widmark: the curve is C₀ − β × τ from the start.
        let widmark = ref_bac_at(0.0, 0.0, 0.5);
        let expected = initial_bac_g_per_l(grams, ref_body_water_litres())
            - ELIMINATION_RATE_G_PER_L_PER_H * 0.5;
        assert!(
            (widmark - expected).abs() < DERIVED,
            "{widmark} instead of {expected}"
        );
    }

    #[test]
    fn equal_durations_degenerate_into_an_isosceles_triangle() {
        // §6.4: a = b leaves no plateau — m = M — and the rate is symmetric
        // about τ = a.
        let grams = ref_alcohol_grams();
        let (a, b) = (0.25, 0.25);
        let apex = absorption_rate_g_per_h(grams, a, b, a);
        assert!((apex - grams / a).abs() < DERIVED, "apex: {apex} g/h");
        for offset in [0.05, 0.1, 0.2] {
            let before = absorption_rate_g_per_h(grams, a, b, a - offset);
            let after = absorption_rate_g_per_h(grams, a, b, a + offset);
            assert!(
                (before - after).abs() < 1e-9,
                "not symmetric at ±{offset} h: {before} vs {after}"
            );
        }
        // Half of the dose is absorbed at the apex, by symmetry.
        let absorbed = absorbed_alcohol_grams(grams, a, b, a);
        assert!(
            (absorbed - grams / 2.0).abs() < DERIVED,
            "at the apex: {absorbed} g of {grams} g"
        );
    }

    // ----- §10.1 — shape of the curve before the peak ----------------------
    //
    // Everything below describes the concentration curve *before* its peak,
    // which is exactly what the absorption model decides. Written against the
    // trapezoid, the current default, and kept ignored until #37 settles the
    // model. Removing the attribute is the act that closes #37.

    #[test]
    #[ignore = "modèle d'absorption non arbitré — cf. #37"]
    fn the_trapezoid_peaks_at_0_227_g_per_l_after_45_7_minutes() {
        // The peak is where the concentration stops growing, i.e. where
        // κ × R(τ) = β. With κ = 0.806 / TBW = 0.017 590 495 021 846 262,
        // that is R = 8.527 332 506 203 473 g/h, reached on the falling branch
        // — R peaks at 39.45 g/h, far above — hence
        //   τ* = a + b − β × a × b / (κ × A) = 0.761 281 516 635 374 h
        //      = 45.676 890 998 122 44 min.
        let grams = ref_alcohol_grams();
        let kappa = initial_bac_g_per_l(1.0, ref_body_water_litres());
        let rate_at_peak = ELIMINATION_RATE_G_PER_L_PER_H / kappa;
        let peak_hours = REF_INGESTION_HOURS + REF_ABSORPTION_HOURS
            - rate_at_peak * REF_INGESTION_HOURS * REF_ABSORPTION_HOURS / grams;

        // The instant is the one where the rate of appearance meets β.
        let rate =
            absorption_rate_g_per_h(grams, REF_INGESTION_HOURS, REF_ABSORPTION_HOURS, peak_hours);
        assert!(
            (kappa * rate - ELIMINATION_RATE_G_PER_L_PER_H).abs() < 1e-12,
            "κ × R = {} g/L/h at the claimed peak",
            kappa * rate
        );
        assert!(
            (peak_hours - 0.761_281_516_635_374).abs() < DERIVED,
            "unexpected peak instant: {peak_hours} h"
        );
        // §10.1 quotes 45.7 min: half a unit of the first decimal of a minute.
        assert!(
            (peak_hours * 60.0 - 45.7).abs() < 0.05,
            "disagrees with SPEC.md §10.1: {} min",
            peak_hours * 60.0
        );

        let peak = ref_bac_at(REF_INGESTION_HOURS, REF_ABSORPTION_HOURS, peak_hours);
        assert!(
            (peak - 0.227_376_400_558_264_47).abs() < DERIVED,
            "unexpected peak: {peak} g/L"
        );
        // §10.1 quotes 0.227 g/L.
        assert!(
            (peak - 0.227).abs() < SPEC_THIRD_DECIMAL,
            "disagrees with SPEC.md §10.1: {peak} g/L"
        );
        // And it really is a maximum.
        for offset in [1e-3, 1e-2, 0.1] {
            let before = ref_bac_at(
                REF_INGESTION_HOURS,
                REF_ABSORPTION_HOURS,
                peak_hours - offset,
            );
            let after = ref_bac_at(
                REF_INGESTION_HOURS,
                REF_ABSORPTION_HOURS,
                peak_hours + offset,
            );
            assert!(before < peak, "higher {offset} h before the peak: {before}");
            assert!(after < peak, "higher {offset} h after the peak: {after}");
        }
    }

    #[test]
    #[ignore = "modèle d'absorption non arbitré — cf. #37"]
    fn the_trapezoid_climbs_through_known_values_before_its_peak() {
        // Halfway to the peak, τ*/2 = 0.380 640 758 317 687 h ≈ 22.838 min:
        //   κ × absorbed(τ*/2) − β × τ*/2 = 0.091 390 143 405 305 68 g/L.
        let half_way = 0.761_281_516_635_374 / 2.0;
        let bac = ref_bac_at(REF_INGESTION_HOURS, REF_ABSORPTION_HOURS, half_way);
        assert!(
            (bac - 0.091_390_143_405_305_68).abs() < DERIVED,
            "halfway to the peak: {bac} g/L"
        );
        // And at the two corners of the trapezoid, τ = m = 20 min and
        // τ = M = 30 min.
        let at_m = ref_bac_at(
            REF_INGESTION_HOURS,
            REF_ABSORPTION_HOURS,
            REF_INGESTION_HOURS,
        );
        assert!(
            (at_m - 0.065_657_504_768_639_2).abs() < DERIVED,
            "end of the rise: {at_m} g/L"
        );
        let at_big_m = ref_bac_at(
            REF_INGESTION_HOURS,
            REF_ABSORPTION_HOURS,
            REF_ABSORPTION_HOURS,
        );
        assert!(
            (at_big_m - 0.156_315_009_537_278_4).abs() < DERIVED,
            "end of the plateau: {at_big_m} g/L"
        );
    }

    #[test]
    #[ignore = "modèle d'absorption non arbitré — cf. #37"]
    fn the_linear_ramp_variant_peaks_at_0_222_g_per_l_after_50_minutes() {
        // The ramp of §10.1 spreads the dose uniformly over t_ing + t_abs,
        // which is the a = 0 degeneracy of the trapezoid. The rate then stays
        // above β / κ for the whole ramp, so the peak is at its very end:
        //   C₀ − β × (a + b) = 0.346 972 514 305 917 5 − 0.125
        //                    = 0.221 972 514 305 917 52 g/L.
        let ramp_hours = REF_INGESTION_HOURS + REF_ABSORPTION_HOURS;
        let peak = ref_bac_at(0.0, ramp_hours, ramp_hours);
        assert!(
            (ramp_hours * 60.0 - 50.0).abs() < 1e-9,
            "unexpected peak instant: {} min",
            ramp_hours * 60.0
        );
        assert!(
            (peak - 0.221_972_514_305_917_52).abs() < DERIVED,
            "unexpected peak: {peak} g/L"
        );
        // Rounded to the third decimal this is 0.222 g/L, the figure SPEC.md
        // §10.1 quotes — and not the 0.221 g/L found elsewhere, which is a
        // truncation of the same number.
        assert!(
            (peak - 0.222).abs() < SPEC_THIRD_DECIMAL,
            "disagrees with SPEC.md §10.1: {peak} g/L"
        );
        assert!(
            (peak - 0.221).abs() > SPEC_THIRD_DECIMAL,
            "0.221 g/L would round to this: {peak} g/L"
        );
    }

    #[test]
    #[ignore = "modèle d'absorption non arbitré — cf. #37"]
    fn the_pure_widmark_variant_peaks_at_c_zero_at_the_ingestion_time() {
        // Both durations at zero: the whole dose is in the blood at τ = 0, and
        // the curve only ever falls afterwards.
        let peak = ref_bac_at(0.0, 0.0, 0.0);
        assert!(
            (peak - 0.346_972_514_305_917_5).abs() < DERIVED,
            "unexpected peak: {peak} g/L"
        );
        assert!(
            (peak - 0.347).abs() < SPEC_THIRD_DECIMAL,
            "disagrees with SPEC.md §10.1: {peak} g/L"
        );
        assert!(
            ref_bac_at(0.0, 0.0, 0.1) < peak,
            "the pure Widmark curve did not start falling immediately"
        );
    }
}
