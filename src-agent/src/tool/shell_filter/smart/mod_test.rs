#![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn cd_prefix_matches_cargo_build() {
        let raw = "\
   Compiling foo v0.1.0
   Compiling bar v0.2.0
    Finished dev [unoptimized + debuginfo] target(s) in 1.00s
";
        let outcome = try_smart("cd /x && cargo build", raw, Some(0));
        let outcome = outcome.expect("cd-prefixed cargo build should be recognized");
        assert_eq!(outcome.filter_name, Some("cargo-build"));
        assert!(outcome.changed);
    }

    #[test]
    fn unrelated_command_returns_none() {
        assert!(try_smart("echo hi", "hi\n", Some(0)).is_none());
    }
