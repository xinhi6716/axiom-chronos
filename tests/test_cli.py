"""Exercise the axiom CLI against a scripted claude that records how it was launched."""
import ast
import io
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / 'src'))

from axiom import cli, doctor, session  # noqa: E402
from axiom.home import ENGINE  # noqa: E402

FAKE_CLAUDE = json.dumps([sys.executable, '-I', str(HERE / 'fakes' / 'fake_claude.py')])


class EngineContractTests(unittest.TestCase):
    def test_dispatcher_reads_the_shipped_policy(self):
        dispatcher = (ENGINE / 'agents' / 'dispatcher.md').read_text(encoding='utf-8')
        reference = re.search(r'Read `\$\{CLAUDE_PLUGIN_ROOT\}/([^`]+)`', dispatcher)
        self.assertIsNotNone(reference, 'dispatcher must name its plugin-relative policy')
        self.assertTrue((ENGINE / reference.group(1)).is_file(), reference.group(1))

    def test_documented_python_minimum_matches_package(self):
        metadata = (HERE.parent / 'pyproject.toml').read_text(encoding='utf-8')
        minimum = re.search(r'^requires-python = ">=(\d+\.\d+)"$', metadata, re.MULTILINE).group(1)
        for name in ('README.md', 'README.zh-TW.md', 'docs/bridge.md'):
            with self.subTest(document=name):
                document = (HERE.parent / name).read_text(encoding='utf-8')
                self.assertIn(f'Python {minimum}', document)

    def test_sources_parse_at_declared_python_minimum(self):
        # This checks grammar, not runtime API compatibility or live integrations.
        metadata = (HERE.parent / 'pyproject.toml').read_text(encoding='utf-8')
        minimum = re.search(r'^requires-python = ">=(\d+\.\d+)"$', metadata, re.MULTILINE).group(1)
        version = tuple(map(int, minimum.split('.')))
        for source in (HERE.parent / 'src').rglob('*.py'):
            with self.subTest(source=source.relative_to(HERE.parent)):
                ast.parse(source.read_text(encoding='utf-8'), filename=str(source), feature_version=version)


class WishTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.project = self.root / 'todo-app'
        self.project.mkdir()
        self.record = self.root / 'record.json'
        env = patch.dict(os.environ, {'AXIOM_CLAUDE_CMD': FAKE_CLAUDE, 'AXIOM_HOME': str(self.root / 'home'),
                                      'FAKE_CLAUDE_RECORD': str(self.record)})
        env.start()
        self.addCleanup(env.stop)

    def wish(self, scenario='ok'):
        out = io.StringIO()
        with patch.dict(os.environ, {'FAKE_CLAUDE_SCENARIO': scenario}):
            status = session.run('wish', '幫 todo 加統計功能', str(self.project), out=out)
        return status, out.getvalue()

    def test_launch_loads_only_the_engine(self):
        self.assertEqual(self.wish()[0], 0)
        launch = json.loads(self.record.read_text(encoding='utf-8'))
        argv = launch['argv']
        self.assertEqual(argv[argv.index('-p') + 1], '/chronos:wish 幫 todo 加統計功能')
        self.assertEqual(Path(argv[argv.index('--plugin-dir') + 1]), ENGINE)
        self.assertEqual(argv[argv.index('--setting-sources') + 1], 'local')
        self.assertIn('mcp__plugin_chronos_bridge__codex_run', argv[argv.index('--allowedTools') + 1].split(','))
        self.assertEqual(argv[argv.index('--model') + 1], 'opus')
        self.assertEqual(Path(launch['cwd']), self.project)
        self.assertEqual(launch['env'], {'AXIOM_DATA_HOME': str(self.root / 'home'), 'AXIOM_PYTHON': sys.executable})
        self.assertTrue((ENGINE / '.claude-plugin' / 'plugin.json').is_file())

    def test_progress_shows_steps_and_the_answer(self):
        _, text = self.wish()
        self.assertIn('> chronos:dispatcher: Dispatch wish', text)
        self.assertIn('  - orch unit add u1', text)
        self.assertIn('  - codex terra: todo.py defines stats(items).', text)
        self.assertNotIn('Read', text)
        self.assertIn('< chronos:dispatcher finished', text)
        self.assertIn('統計功能已完成。', text)
        self.assertIn('$0.36', text)
        logs = list((self.root / 'home' / 'logs').glob('*-wish.jsonl'))
        self.assertEqual(len(logs), 1)
        self.assertIn('"type": "result"', logs[0].read_text(encoding='utf-8'))

    def test_failures_are_explicit(self):
        self.assertEqual(self.wish('error')[0], 1)
        status, text = self.wish('crash')
        self.assertEqual(status, 3)
        self.assertIn('without a result', text)
        self.assertIn('MCP server is failed', self.wish('no-bridge')[1])

    def test_wish_text_may_name_flags(self):
        with patch.dict(os.environ, {'FAKE_CLAUDE_SCENARIO': 'ok'}), patch('sys.stdout', io.StringIO()):
            status = cli.main(['wish', '--project', str(self.project), '在', 'cli.py', '加', '--pending', '參數'])
        self.assertEqual(status, 0)
        argv = json.loads(self.record.read_text(encoding='utf-8'))['argv']
        self.assertEqual(argv[argv.index('-p') + 1], '/chronos:wish 在 cli.py 加 --pending 參數')

    def test_no_architect_marks_the_wish(self):
        with patch.dict(os.environ, {'FAKE_CLAUDE_SCENARIO': 'ok'}), patch('sys.stdout', io.StringIO()):
            self.assertEqual(cli.main(['wish', '--project', str(self.project), '--no-architect', 'add', 'x']), 0)
        argv = json.loads(self.record.read_text(encoding='utf-8'))['argv']
        self.assertEqual(argv[argv.index('-p') + 1], '/chronos:wish [no-architect] add x')

    def test_research_runs_the_research_skill(self):
        with patch.dict(os.environ, {'FAKE_CLAUDE_SCENARIO': 'ok'}), patch('sys.stdout', io.StringIO()):
            self.assertEqual(cli.main(['research', '--project', str(self.project), 'RRAM', '因果']), 0)
        argv = json.loads(self.record.read_text(encoding='utf-8'))['argv']
        self.assertEqual(argv[argv.index('-p') + 1], '/chronos:research RRAM 因果')
        self.assertEqual(argv[argv.index('--max-turns') + 1], '40')

    def test_missing_claude_is_reported(self):
        with patch.dict(os.environ, {'AXIOM_CLAUDE_CMD': ''}), patch('shutil.which', return_value=None):
            err = io.StringIO()
            with patch('sys.stderr', err):
                self.assertEqual(cli.main(['wish', '--project', str(self.project), 'x']), 2)
        self.assertIn('claude is not on PATH', err.getvalue())


class DoctorTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        fake = json.dumps([sys.executable, '-I', str(HERE / 'fakes' / 'fake_claude.py')])
        env = patch.dict(os.environ, {'AXIOM_HOME': self.temp.name, 'AXIOM_CLAUDE_CMD': fake, 'AXIOM_CODEX_CMD': fake,
                                      'AXIOM_AGY_CMD': fake, 'AXIOM_BUN_CMD': fake,
                                      'AXIOM_PWSH_CMD': json.dumps([r'C:\Program Files\PowerShell\7\pwsh.exe'])})
        env.start()
        self.addCleanup(env.stop)

    def test_all_present_passes(self):
        out = io.StringIO()
        self.assertEqual(doctor.main(out), 0)
        self.assertIn('claude', out.getvalue())
        self.assertIn('trigram', out.getvalue())

    def test_missing_required_tool_fails(self):
        with patch.dict(os.environ, {'AXIOM_CODEX_CMD': ''}), patch('axiom.home.shutil.which', return_value=None):
            out = io.StringIO()
            self.assertEqual(doctor.main(out), 1)
        self.assertIn('ERR  codex', out.getvalue())

    def test_store_pwsh_is_flagged(self):
        store = r'C:\Users\u\AppData\Local\Microsoft\WindowsApps\pwsh.exe'
        with patch.dict(os.environ, {'AXIOM_PWSH_CMD': json.dumps([store])}):
            label, ok, need, detail = doctor.windows_shell()
        self.assertEqual((ok, need), (False, doctor.REQUIRED))
        self.assertIn('--installer-type wix', detail)


if __name__ == '__main__':
    unittest.main()
