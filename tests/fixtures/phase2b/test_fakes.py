"""Subprocess-level coverage of fake CLI script and log options."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
CODEX = ROOT / 'tests/fakes/fake_codex.py'
MEMBER = ROOT / 'tests/fakes/fake_member.py'


class FakeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)

    def script(self, entries):
        path = self.directory / 'script.json'
        path.write_text(json.dumps(entries), encoding='utf-8')
        return path

    def codex(self, prompts, script, log=None):
        requests = [
            {'id': 1, 'method': 'initialize', 'params': {}},
            {'id': 2, 'method': 'thread/start', 'params': {'model': 'test-model'}},
        ]
        for index, (prompt, schema) in enumerate(prompts, 3):
            params = {'threadId': 'thread-1', 'input': [{'type': 'text', 'text': prompt}], 'effort': 'high'}
            if schema:
                params['outputSchema'] = {'type': 'object'}
            requests.append({'id': index, 'method': 'turn/start', 'params': params})
        args = [sys.executable, str(CODEX), 'app-server', f'--fake-script={script}']
        if log:
            args.append(f'--fake-log={log}')
        env = os.environ.copy()
        env.pop('FAKE_CODEX_LOG', None)
        proc = subprocess.run(args, input=''.join(json.dumps(request) + '\n' for request in requests),
                              text=True, encoding='utf-8', capture_output=True, check=True, timeout=10, env=env)
        events = [json.loads(line) for line in proc.stdout.splitlines()]
        return [event['params']['item']['text'] for event in events
                if event.get('method') == 'item/completed' and event['params']['item']['type'] == 'agentMessage']

    def member(self, prompt, script, fable):
        args = [sys.executable, str(MEMBER), f'--fake-script={script}']
        if fable:
            args.append('--strict-mcp-config')
        else:
            args.append(prompt)
        proc = subprocess.run(args, input=prompt if fable else '', text=True, encoding='utf-8',
                              capture_output=True, check=True, timeout=10)
        if fable:
            events = [json.loads(line) for line in proc.stdout.splitlines()]
            result = next(event for event in events if event['type'] == 'result')
            usage = [event['message']['usage'] for event in events if event['type'] == 'assistant']
            return json.loads(result['result']), usage
        result = json.loads(proc.stdout)
        return json.loads(result['result']), result['usage']

    def test_codex_script_and_fallback(self):
        script = self.script([{'match': 'needle', 'reply': {'answer': 'first'}},
                              {'match': 'needle', 'reply': {'answer': 'second'}},
                              {'match': 'literal', 'reply': 'plain reply'},
                              {'match': 'CRASH', 'reply': {'answer': 'survived'}}])
        replies = self.codex([('needle', False), ('needle schema', True), ('literal', False),
                              ('CRASH', False), ('absent', False), ('absent schema', True)], script)
        self.assertEqual(json.loads(replies[0]), {'answer': 'first'})
        self.assertEqual(json.loads(replies[1]), {'answer': 'first'})
        self.assertEqual(replies[2], 'plain reply')
        self.assertEqual(json.loads(replies[3]), {'answer': 'survived'})
        self.assertEqual(json.loads(replies[4])['brief'], 'absent')
        self.assertEqual(json.loads(replies[5]), {'answer': 'ok'})

    def test_codex_log(self):
        script = self.script([{'match': 'missing', 'reply': 'unused'}])
        log = self.directory / 'calls.jsonl'
        replies = self.codex([('normal', False)], script, log)
        self.assertEqual(json.loads(replies[0])['brief'], 'normal')
        records = [json.loads(line) for line in log.read_text(encoding='utf-8').splitlines()]
        self.assertEqual([record['method'] for record in records[:3]], ['initialize', 'thread/start', 'turn/start'])
        self.assertEqual(records[1]['params']['model'], 'test-model')
        self.assertEqual(records[2]['params']['effort'], 'high')
        self.assertEqual(records[2]['params']['input'][0]['text'], 'normal')
        self.assertFalse(Path(str(log) + '.lock').exists())

    def test_member_script_and_fallback(self):
        script = self.script([{'match': 'needle', 'reply': {'summary': 'first', 'value': 42}},
                              {'match': 'needle', 'reply': {'summary': 'second'}}])
        for fable in (True, False):
            with self.subTest(fable=fable):
                answer, usage = self.member('a needle prompt', script, fable)
                self.assertEqual(answer, {'summary': 'first', 'value': 42})
                if fable:
                    self.assertTrue(usage)
                    self.assertEqual(usage[0]['input_tokens'], 1000)
                else:
                    self.assertEqual(usage['total_tokens'], 1234)
                answer, usage = self.member('absent', script, fable)
                self.assertEqual(answer['summary'], 'fable summary' if fable else 'argon summary')
                self.assertTrue(usage)


if __name__ == '__main__':
    unittest.main()
