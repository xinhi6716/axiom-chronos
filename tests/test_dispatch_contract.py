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

    def test_dispatcher_cannot_pass_work_itself(self):
        r1 = 'A unit whose review did not finish, because `review_code` and the `chronos:reviewer` fallback both failed or never ran, is `"unreviewed"`, never `"verified"`.'
        r2 = 'You never dismiss a blocker or major finding yourself: a unit still covered by it after the one fix is `"review-failed"`, and your report lists the finding for Opus to rule on.'
        dispatcher_body = body(self.dispatcher)
        holds = lambda text: r1 in text and r2 in text
        self.assertTrue(holds(dispatcher_body))
        self.assertFalse(holds(dispatcher_body.replace(r1, '')))
        self.assertFalse(holds(dispatcher_body.replace(r2, '')))

    def test_brief_path_comes_from_unit_id(self):
        memory_keeper = (AGENTS / 'memory-keeper.md').read_text(encoding='utf-8')
        dispatcher_body = body(self.dispatcher)
        keeper_body = body(memory_keeper)
        registration = '"--brief", "<store>/briefs/<unit>.md"'
        missing_brief = "If its brief is missing, say so for that unit and never read another unit's brief in its place."
        old_path = 'path in the `brief` column'

        registration_holds = lambda text: registration in text
        missing_brief_holds = lambda text: missing_brief in text
        old_path_absent = lambda text: old_path not in text
        self.assertTrue(registration_holds(dispatcher_body))
        self.assertFalse(registration_holds(dispatcher_body.replace(registration, '')))
        self.assertIn('briefs/<unit>.md', keeper_body)
        self.assertTrue(missing_brief_holds(keeper_body))
        self.assertFalse(missing_brief_holds(keeper_body.replace(missing_brief, '')))
        self.assertTrue(old_path_absent(keeper_body))
        self.assertFalse(old_path_absent(keeper_body + old_path))


if __name__ == '__main__':
    unittest.main()
