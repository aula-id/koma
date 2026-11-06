"""Effect classification rules, including the validator over the schema."""
import unittest

from koma_sec_daemon import effects


class EffectOfTest(unittest.TestCase):
    def test_unknown_tool_has_no_effect(self):
        self.assertIsNone(effects.effect_of("sec_nope"))

    def test_argument_dependent_row_drops_without_its_parameter(self):
        # `sec_remote` without a host must not be admitted as network-capable.
        self.assertEqual(effects.effect_of("sec_remote", {"host": "a.example"}), effects.REMOTE)
        self.assertEqual(effects.effect_of("sec_remote", {}), effects.LOCAL)

    def test_net_is_not_the_same_axis_as_effect(self):
        # A single request is still `remote`; payload firing is `exploit`.
        self.assertEqual(effects.effect_of("sec_http", {"url": "u"}), effects.REMOTE)
        self.assertEqual(effects.effect_of("sec_sqlmap", {"url": "u"}), effects.EXPLOIT)

    def test_destination_prefers_the_first_declared_parameter(self):
        self.assertEqual(effects.destination_of({"url": "u", "host": "h"}), "u")
        self.assertIsNone(effects.destination_of({"url": "   "}))
        self.assertIsNone(effects.destination_of(None))


class ManifestTest(unittest.TestCase):
    def test_every_row_uses_known_labels(self):
        for name, (effect, exec_, depends_on, _note) in effects.MANIFEST.items():
            with self.subTest(tool=name):
                self.assertIn(effect, effects.EFFECTS)
                self.assertIn(exec_, effects.EXECS)
                if depends_on:
                    self.assertIn(depends_on, effects.DESTINATION_PARAMS)

    def test_network_effects_require_a_network_exec(self):
        # An exploit running without the egress path is a routing bug.
        for name, (effect, exec_, _dep, _note) in effects.MANIFEST.items():
            if effect in (effects.REMOTE, effects.EXPLOIT):
                with self.subTest(tool=name):
                    self.assertIn(exec_, (effects.NET, effects.BROWSER))

    def test_no_unclassified_tool_ships(self):
        # Every tool the restored registry knows must have a row, so a new tool
        # cannot default into an unlabelled state.
        expected = {
            "sec_http", "sec_remote", "sec_sqlmap", "sec_nuclei", "sec_ffuf",
            "sec_dalfox", "sec_zap", "sec_xss_confirm", "sec_triage",
            "sec_sourcemap", "sec_wasm", "sec_rop", "sec_crack", "sec_z3",
            "sec_sage", "sec_rsa", "sec_factor", "sec_lattice", "sec_hashid",
            "sec_decode", "sec_jsdeobf", "sec_unmin", "sec_pwntmpl",
        }
        self.assertEqual(set(effects.MANIFEST), expected)


if __name__ == "__main__":
    unittest.main()
