"""The metadata validator, and the reconciled registry."""
import unittest

from koma_sec_daemon import effects, metadata, registry


def desc(name="sec_http", **over):
    base = {
        "name": name,
        "description": "d",
        "parameters": {
            "type": "object",
            "properties": {"url": {"type": "string"}},
            "required": ["url"],
        },
        "effect": "remote",
        "exec": "net",
        "domain": "web",
        "handler": lambda a, s: "",
    }
    base.update(over)
    return base


class ValidateTest(unittest.TestCase):
    def test_a_correct_descriptor_passes(self):
        self.assertEqual(metadata.validate(desc()), [])

    def test_unclassified_tool_is_refused(self):
        self.assertTrue(metadata.validate(desc(name="sec_unknown")))

    def test_effect_contradicting_the_manifest_is_refused(self):
        reasons = metadata.validate(desc(effect="static"))
        self.assertTrue(any("disagrees with manifest" in r for r in reasons))

    def test_unknown_effect_label_is_refused(self):
        reasons = metadata.validate(desc(effect="spicy"))
        self.assertTrue(any("unknown effect" in r for r in reasons))

    def test_remote_without_a_destination_is_refused(self):
        reasons = metadata.validate(
            desc(parameters={"type": "object", "properties": {"n": {"type": "string"}}})
        )
        self.assertTrue(any("no destination" in r for r in reasons))

    def test_fixed_host_tool_needs_no_destination(self):
        reasons = metadata.validate(
            desc(
                name="sec_factor",
                effect="remote",
                parameters={"type": "object", "properties": {"n": {"type": "string"}}},
            )
        )
        self.assertEqual(reasons, [])

    def test_a_legacy_descriptor_is_derived_and_checked(self):
        # No effect/exec: derived from risk/compute, and the derivation must still
        # agree with the manifest.
        legacy = desc()
        legacy.pop("effect")
        legacy.pop("exec")
        legacy["risk"] = True
        legacy["compute"] = "network"
        self.assertEqual(metadata.validate(legacy), [])

    def test_a_legacy_descriptor_that_derives_wrongly_is_refused(self):
        legacy = desc()
        legacy.pop("effect")
        legacy.pop("exec")
        legacy["risk"] = False  # would derive to `local`, manifest says remote
        legacy["compute"] = "network"
        self.assertTrue(any("legacy risk maps to" in r for r in metadata.validate(legacy)))


class ReconciledRegistryTest(unittest.TestCase):
    """The real registry must be fully classified, not just the fixtures."""

    def test_no_restored_tool_is_refused(self):
        self.assertEqual(registry.refused_descriptors(), [])

    def test_every_shipped_descriptor_uses_the_new_vocabulary(self):
        for d in registry.descriptors():
            with self.subTest(tool=d["name"]):
                self.assertIn(d["effect"], effects.EFFECTS)
                self.assertIn(d["exec"], effects.EXECS)
                # Legacy keys must not leak onto the wire.
                self.assertNotIn("risk", d)
                self.assertNotIn("compute", d)

    def test_migrated_tool_matches_the_manifest(self):
        for d in registry.descriptors():
            with self.subTest(tool=d["name"]):
                row = effects.MANIFEST[d["name"]]
                self.assertEqual((d["effect"], d["exec"]), (row[0], row[1]))


if __name__ == "__main__":
    unittest.main()
