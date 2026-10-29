"""Check Astra's coverage contract and its presentation in wish."""
import copy
import json
from pathlib import Path
import sys
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / 'src'))
ENGINE = HERE.parent / 'src' / 'axiom' / 'engine'


def coverage_ok(schema):
    expected = {
        'type': 'object',
        'additionalProperties': False,
        'required': ['read', 'not_read'],
        'properties': {
            'read': {'type': 'array', 'items': {'type': 'string'}},
            'not_read': {'type': 'array', 'items': {'type': 'string'}},
        },
    }
    return (schema.get('required') == ['summary', 'coverage', 'changes', 'out_of_scope']
            and set(schema.get('required', [])) == set(schema.get('properties', {}))
            and schema.get('properties', {}).get('coverage') == expected)


class AstraCoverageTests(unittest.TestCase):
    def test_coverage_schema_and_negative_control(self):
        schema = json.loads((ENGINE / 'research' / 'audit.schema.json').read_text(encoding='utf-8'))
        self.assertTrue(coverage_ok(schema))
        missing_required = copy.deepcopy(schema)
        missing_required['required'].remove('coverage')
        self.assertFalse(coverage_ok(missing_required))

    def test_wish_carries_coverage_and_review_states(self):
        wish = (ENGINE / 'skills' / 'wish' / 'SKILL.md').read_text(encoding='utf-8')
        for phrase in ('coverage.read', 'coverage.not_read', '## Coverage'):
            with self.subTest(phrase=phrase):
                self.assertIn(phrase, wish)
        step7 = wish.split('\n7. ', 1)[1]
        for state in ('review-failed', 'unreviewed'):
            with self.subTest(state=state):
                self.assertIn(state, step7)


if __name__ == '__main__':
    unittest.main()
