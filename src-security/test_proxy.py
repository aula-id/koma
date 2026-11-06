"""Egress proxy rules: the vectors must match `app::sec::scope::Host::admits`."""
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "proxy"))

from koma_proxy import Rules, _parse, _rule_admits, _split_authority  # noqa: E402


def rules_from(text: str) -> Rules:
    fd, path = tempfile.mkstemp()
    with os.fdopen(fd, "w") as fh:
        fh.write(text)
    r = Rules(path)
    r.load()
    return r


class RuleAdmitTest(unittest.TestCase):
    def test_exact_host_any_port(self):
        self.assertTrue(_rule_admits("a.example", None, "a.example", 443))

    def test_subdomain_is_covered_but_lookalike_is_not(self):
        self.assertTrue(_rule_admits("example.com", None, "api.example.com", 443))
        self.assertFalse(_rule_admits("example.com", None, "notexample.com", 443))

    def test_leading_dot_is_subdomains_only(self):
        self.assertTrue(_rule_admits(".example.com", None, "api.example.com", 443))
        self.assertFalse(_rule_admits(".example.com", None, "example.com", 443))

    def test_ip_never_suffix_matches(self):
        self.assertTrue(_rule_admits("10.0.0.1", None, "10.0.0.1", 80))
        self.assertFalse(_rule_admits("10.0.0.1", None, "20.10.0.1", 80))

    def test_port_spec_narrows(self):
        ports = frozenset({8080})
        self.assertTrue(_rule_admits("h", ports, "h", 8080))
        self.assertFalse(_rule_admits("h", ports, "h", 80))


class RulesFileTest(unittest.TestCase):
    def test_comments_and_blanks_are_ignored(self):
        self.assertEqual(_parse_from("# c\n\na.example *\n"), [("a.example", None)])

    def test_a_bad_line_is_skipped_not_fatal(self):
        self.assertEqual(_parse_from("good.example *\nbroken\n"), [("good.example", None)])

    def test_a_missing_file_denies_everything(self):
        r = Rules("/nonexistent/rules")
        r.load()
        self.assertFalse(r.allows("a.example", 80))

    def test_the_file_is_reloaded_when_it_changes(self):
        r = rules_from("a.example *\n")
        self.assertTrue(r.allows("a.example", 80))
        with open(r.path, "w") as fh:
            fh.write("b.example *\n")
        os.utime(r.path, ns=(1, 1))
        self.assertTrue(r.allows("b.example", 80))
        self.assertFalse(r.allows("a.example", 80))

    def test_a_port_spec_parses(self):
        r = rules_from("a.example 80,443\n")
        self.assertTrue(r.allows("a.example", 80))
        self.assertFalse(r.allows("a.example", 8080))

    def test_host_case_is_folded(self):
        self.assertTrue(rules_from("A.Example *\n").allows("a.example", 1))


class AuthorityTest(unittest.TestCase):
    def test_host_and_port(self):
        self.assertEqual(_split_authority("h:8080", 443), ("h", 8080))

    def test_default_port_when_absent(self):
        self.assertEqual(_split_authority("h", 443), ("h", 443))

    def test_ipv6_brackets(self):
        self.assertEqual(_split_authority("[::1]:8443", 443), ("::1", 8443))
        self.assertEqual(_split_authority("[::1]", 443), ("::1", 443))

    def test_a_bare_ipv6_is_not_split_on_a_colon(self):
        # Two or more colons with no brackets: the whole thing is the host.
        self.assertEqual(_split_authority("fe80::1", 443), ("fe80::1", 443))


def _parse_from(text: str):
    fd, path = tempfile.mkstemp()
    with os.fdopen(fd, "w") as fh:
        fh.write(text)
    try:
        return _parse(path)
    finally:
        os.unlink(path)


if __name__ == "__main__":
    unittest.main()
