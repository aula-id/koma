use super::*;

#[test]
fn the_provenance_verdicts_parse() {
    for (raw, want) in [
        (
            r#"{"verdict":"disproved","excerpt":"x"}"#,
            Verdict::Disproved,
        ),
        (r#"{"verdict":"unproven","excerpt":"x"}"#, Verdict::Unproven),
        (r#"{"verdict":"untested","excerpt":"x"}"#, Verdict::Untested),
    ] {
        assert_eq!(parse(raw).verdict, want, "{raw}");
    }
}

#[test]
fn only_a_confirmation_establishes_a_finding() {
    // Pinned: the provenance verdicts must NOT quietly count as proof.
    for v in [Verdict::Disproved, Verdict::Unproven, Verdict::Untested] {
        assert!(!v.establishes_finding(), "{v:?}");
    }
    assert!(Verdict::Confirmed.establishes_finding());
}
