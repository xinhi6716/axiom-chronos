"""Check the engine's internal references: every plugin-root path exists and every tool an agent names is served."""
import json
from pathlib import Path
import re
import sys
import unittest

HERE = Path(__file__).resolve().parent
ENGINE = HERE.parent / 'src' / 'axiom' / 'engine'
sys.path.insert(0, str(ENGINE / 'bridge'))
sys.path.insert(0, str(HERE.parent / 'src'))

from axiom import session  # noqa: E402
from server import TOOLS  # noqa: E402

PLUGIN = json.loads((ENGINE / '.claude-plugin' / 'plugin.json').read_text(encoding='utf-8'))
TEXT_FILES = [p for p in ENGINE.rglob('*') if p.suffix in ('.md', '.json') and 'vendor' not in p.parts]


class EngineReferenceTests(unittest.TestCase):
    def test_plugin_root_paths_exist(self):
        for path in TEXT_FILES:
            for ref in re.findall(r'\$\{CLAUDE_PLUGIN_ROOT\}/([^\s`"\')]+)', path.read_text(encoding='utf-8')):
                with self.subTest(file=path.relative_to(ENGINE).as_posix(), ref=ref):
                    self.assertTrue((ENGINE / ref).is_file())

    def test_named_bridge_tools_are_served(self):
        prefix = f'mcp__plugin_{PLUGIN["name"]}_bridge__'
        served = {tool['name'] for tool in TOOLS}
        for path in TEXT_FILES:
            for tool in re.findall(r'mcp__plugin_[a-z0-9-]+_[a-z0-9-]+__([a-z_]+)', path.read_text(encoding='utf-8')):
                with self.subTest(file=path.relative_to(ENGINE).as_posix(), tool=tool):
                    self.assertIn(tool, served)
            self.assertNotRegex(path.read_text(encoding='utf-8'), r'mcp__plugin_(?!' + re.escape(PLUGIN['name']) + r'_bridge__)[a-z0-9-]+_[a-z0-9-]+__')
        self.assertTrue(session.BRIDGE == prefix)
        self.assertEqual({t[len(prefix):] for t in session.TOOLS if t.startswith(prefix)} - served, set())

    def test_named_agents_exist(self):
        agents = {re.search(r'^name:\s*(\S+)', p.read_text(encoding='utf-8'), re.M).group(1) for p in (ENGINE / 'agents').glob('*.md')}
        for path in TEXT_FILES:
            for agent in re.findall(re.escape(PLUGIN['name']) + r':([a-z-]+)', path.read_text(encoding='utf-8')):
                if agent in ('bridge',):
                    continue
                with self.subTest(file=path.relative_to(ENGINE).as_posix(), agent=agent):
                    self.assertTrue(agent in agents or (ENGINE / 'skills' / agent).is_dir())


if __name__ == '__main__':
    unittest.main()
