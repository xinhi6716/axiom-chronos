"""Check that the dispatcher marks a unit verified only after its review."""
from pathlib import Path
import sys
import unittest

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / 'src'))
AGENTS = HERE.parent / 'src' / 'axiom' / 'engine' / 'agents'


def body(text):
    return text.split('---', 2)[2]


def review_gate_holds(text) -> bool:
    text = body(text)
    review = text.find('review_code')
    built = text.find('"--state", "built"')
    verified = text.find('"--state", "verified"')
    return 0 <= built < review < verified


class DispatchContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.dispatcher = (AGENTS / 'dispatcher.md').read_text(encoding='utf-8')

    def test_review_gate_and_negative_control(self):
        self.assertTrue(review_gate_holds(self.dispatcher))
        broken = self.dispatcher.replace('"--state", "built"', '"--state", "verified"')
        self.assertFalse(review_gate_holds(broken))


if __name__ == '__main__':
    unittest.main()
