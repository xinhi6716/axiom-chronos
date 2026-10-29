"""Check headless session progress and Codex reporting."""
import io
import json
import os
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / 'src'))

from axiom import session


class SessionProgressTests(unittest.TestCase):
    def setUp(self):
        self.out = io.StringIO()
        self.renderer = session.Renderer(self.out)

    def call(self, ident, brief, role='astra', parent=None, name=None):
        self.renderer.event({'type': 'assistant', 'parent_tool_use_id': parent, 'message': {'content': [
            {'type': 'tool_use', 'id': ident, 'name': name or session.BRIDGE + 'codex_run',
             'input': {'role': role, 'brief': brief}}]}})

    def tool_result(self, ident, content):
        self.renderer.event({'type': 'user', 'message': {'content': [
            {'type': 'tool_result', 'tool_use_id': ident, 'content': content}]}})

    def finish(self):
        self.renderer.event({'type': 'result', 'result': 'done', 'duration_ms': 1000, 'total_cost_usd': 0.5})
        return self.out.getvalue()

    def test_command_disallows_powershell(self):
        with patch.dict(os.environ, {'AXIOM_CLAUDE_CMD': json.dumps(['claude'])}):
            argv = session.command('wish', 'test', 'opus', 3)
        self.assertEqual(argv[argv.index('--disallowedTools') + 1], 'PowerShell')
        self.assertNotIn('PowerShell', argv[argv.index('--allowedTools') + 1].split(','))
        self.assertIn('Bash', argv[argv.index('--allowedTools') + 1].split(','))

    def test_labels(self):
        self.call('a', 'context\nGOAL   actual goal\nother')
        self.call('b', '\n  fallback line  \nmore')
        self.call('c', ' \n')
        self.call('d', 'GOAL ' + 'x' * 110)
        self.assertEqual(self.out.getvalue().splitlines(), [
            '- codex astra: actual goal', '- codex astra: fallback line',
            '- codex astra: (no brief)', '- codex astra: ' + 'x' * 100])

    def test_coverage_and_usage(self):
        self.call('a', 'GOAL inspect')
        self.tool_result('a', json.dumps({'model': 'astra', 'structured_output': {'coverage': {
            'read': ['one', 'two'], 'not_read': []}}, 'usage': {
                'inputTokens': 1000, 'cachedInputTokens': 200, 'outputTokens': 30}}))
        self.assertIn('  astra read: one; two\n  astra did not read: (none)\n', self.out.getvalue())
        self.call('b', 'nested', parent='agent')
        report = json.dumps({'model': 'astra', 'usage': {'inputTokens': 50, 'outputTokens': 2},
                             'structured_output': {'coverage': {'read': [], 'not_read': ['three']}}})
        self.tool_result('b', [{'type': 'text', 'text': report[:20]}, {'type': 'image'},
                               {'type': 'text', 'text': report[20:]}])
        self.call('other', '', name='Bash')
        self.tool_result('other', json.dumps({'model': 'astra', 'usage': {'inputTokens': 9999}}))
        text = self.finish()
        self.assertIn('    astra read: (none)\n    astra did not read: three\n', text)
        self.assertIn('codex tokens: astra 1,050 in (200 cached) / 32 out\n', text)

    def test_no_codex_results(self):
        self.call('a', 'GOAL pending')
        self.tool_result('a', 'not json')
        self.assertNotIn('codex tokens:', self.finish())


if __name__ == '__main__':
    unittest.main()
