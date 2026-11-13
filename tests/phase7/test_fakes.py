import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import unittest

ROOT = Path(__file__).resolve().parents[2]
CODEX = ROOT / 'tests/fakes/fake_codex.py'
CLAUDE = ROOT / 'tests/fakes/fake_claude.py'


class Process:
    def __init__(self, script, env, args=(), cwd=None):
        clean = {k: v for k, v in os.environ.items() if not k.startswith('FAKE_')}
        clean.update({k: str(v) for k, v in env.items()})
        self.p = subprocess.Popen([sys.executable, '-I', str(script), *args], cwd=cwd,
                                  env=clean, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE, text=True, encoding='utf-8', bufsize=1)
        self.lines = queue.Queue()
        def read():
            for line in self.p.stdout:
                self.lines.put(json.loads(line))
            self.lines.put(None)
        threading.Thread(target=read, daemon=True).start()

    def send(self, value):
        self.p.stdin.write(json.dumps(value) + '\n')
        self.p.stdin.flush()

    def receive(self, predicate, timeout=5):
        import time
        deadline = time.monotonic() + timeout
        while True:
            try:
                event = self.lines.get(timeout=max(0, deadline - time.monotonic()))
            except queue.Empty:
                raise AssertionError('timed out waiting for event')
            if event is None:
                raise AssertionError('process exited before expected event: ' + self.p.stderr.read())
            if predicate(event):
                return event

    def close(self):
        if self.p.poll() is None:
            self.p.kill()
        self.p.communicate(timeout=5)


class FakeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def process(self, script, env=None, args=()):
        p = Process(script, env or {}, args, self.root)
        self.addCleanup(p.close)
        return p

    def codex(self, env=None, thread=None):
        p = self.process(CODEX, env)
        p.send({'id': 1, 'method': 'thread/start', 'params': thread or {}})
        p.receive(lambda e: e.get('id') == 1)
        return p

    def turn(self, p, text='hello', **params):
        p.send({'id': 2, 'method': 'turn/start', 'params': {'threadId': 'thread-1',
               'input': [{'type': 'text', 'text': text}], **params}})
        return p.receive(lambda e: e.get('method') == 'turn/completed')

    def answer_turn(self, p, text, **params):
        p.send({'id': 2, 'method': 'turn/start', 'params': {'threadId': 'thread-1',
               'input': [{'type': 'text', 'text': text}], **params}})
        event = p.receive(lambda e: e.get('method') == 'item/completed' and
                          e['params']['item']['type'] == 'agentMessage')
        p.receive(lambda e: e.get('method') == 'turn/completed')
        return json.loads(event['params']['item']['text'])

    def claude(self, model='m1', env=None, resume=None):
        args = ['--model', model]
        if resume:
            args += ['--resume', resume]
        return self.process(CLAUDE, {'FAKE_CLAUDE_SCENARIO': 'stages',
                                   'FAKE_CLAUDE_RECORD': self.root / 'record.json', **(env or {})}, args)

    def user(self, p, text='hello'):
        p.send({'type': 'user', 'message': {'role': 'user', 'content': text}})
        return p.receive(lambda e: e.get('type') == 'result')

    def control(self, p, subtype, model='m2'):
        request = {'type': 'control_request', 'request_id': 'req-1',
                   'request': {'subtype': subtype, 'model': model}}
        p.send(request)
        return p.receive(lambda e: e.get('type') == 'control_response'), request

    def test_models_override(self):
        p = self.process(CODEX, {'FAKE_CODEX_MODELS': 'one,two,three'})
        p.send({'id': 1, 'method': 'model/list', 'params': {}})
        result = p.receive(lambda e: e.get('id') == 1)['result']
        self.assertEqual([m['id'] for m in result['data']], ['one', 'two', 'three'])
        self.assertFalse(result.get('nextCursor'))

    def test_params_logged(self):
        log = self.root / 'log'
        thread = {'model': 'm1', 'effort': 'high', 'outputSchema': {'title': 'thread'}}
        turn = {'model': 'm2', 'effort': 'low', 'outputSchema': {'title': 'turn'}}
        p = self.codex({'FAKE_CODEX_LOG': log}, thread)
        self.turn(p, **turn)
        records = [json.loads(line) for line in log.read_text(encoding='utf-8').splitlines()]
        for method, expected in [('thread/start', thread), ('turn/start', turn)]:
            actual = next(r['params'] for r in records if r['method'] == method)
            for key, value in expected.items():
                self.assertEqual(actual[key], value)

    def test_usage_present(self):
        usages = []
        for text in ['identical', 'identical', 'a much longer turn text']:
            p = self.codex()
            usage = self.turn(p, text)['params']['turn']['usage']
            for key in ('inputTokens', 'outputTokens'):
                self.assertIsInstance(usage[key], int)
                self.assertGreater(usage[key], 0)
            usages.append(usage)
        self.assertEqual(usages[0], usages[1])
        self.assertNotEqual(usages[0], usages[2])

    def test_no_usage(self):
        turn = self.turn(self.codex({'FAKE_NO_USAGE': '1'}))['params']['turn']
        self.assertEqual(turn['status'], 'completed')
        self.assertNotIn('usage', turn)
        self.assertIn('usage', self.turn(self.codex())['params']['turn'])

    def test_usage_limit(self):
        turn = self.turn(self.codex({'FAKE_USAGE_LIMIT': '1'}))['params']['turn']
        self.assertEqual(turn['status'], 'failed')
        self.assertEqual(turn['error']['codexErrorInfo'], 'usageLimitExceeded')
        self.assertIn('usage limit', turn['error']['message'].lower())

    def test_hold(self):
        for by_env in (True, False):
            gate = self.root / f'gate-{by_env}'
            p = self.codex({'FAKE_HOLD': gate} if by_env else {})
            text = 'hello' if by_env else f'hello\nFAKE_HOLD {gate}'
            p.send({'id': 2, 'method': 'turn/start', 'params': {'threadId': 'thread-1',
                   'input': [{'type': 'text', 'text': text}]}})
            p.receive(lambda e: e.get('id') == 2)
            with self.assertRaisesRegex(AssertionError, 'timed out'):
                p.receive(lambda e: e.get('method') == 'turn/completed', timeout=0.3)
            gate.touch()
            event = p.receive(lambda e: e.get('method') == 'turn/completed')
            self.assertEqual(event['params']['turn']['status'], 'completed')

    def test_selfcheck_schema(self):
        answer = {'ok': False, 'issues': ['specific']}
        p = self.codex({'FAKE_SELFCHECK': json.dumps(answer), 'FAKE_SELFCHECK_WRITE': 'self.txt'})
        result = self.answer_turn(p, 'FAKE_WRITE forbidden.txt\nDIFF REVIEW',
                                  outputSchema={'title': 'axiom.self_check.v1'})
        self.assertEqual(result, answer)
        self.assertTrue((self.root / 'self.txt').is_file())
        self.assertFalse((self.root / 'forbidden.txt').exists())
        p = self.codex()
        text = 'FAKE_SELFCHECK ' + json.dumps(answer) + '\nFAKE_SELFCHECK_WRITE brief.txt\nFAKE_WRITE forbidden2.txt'
        self.assertEqual(self.answer_turn(p, text, outputSchema={'title': 'axiom.self_check.v1'}), answer)
        self.assertTrue((self.root / 'brief.txt').is_file())
        self.assertFalse((self.root / 'forbidden2.txt').exists())

    def test_review_answer(self):
        answer = {'verdict': 'revise', 'findings': [{'reason': 'specific'}]}
        p = self.codex({'FAKE_REVIEW': json.dumps(answer)}, {'model': 'gpt-6-astra'})
        self.assertEqual(self.answer_turn(p, 'DIFF REVIEW', outputSchema={'title': 'review'}), answer)
        p = self.codex()
        self.assertEqual(self.answer_turn(p, 'DIFF REVIEW\nFAKE_REVIEW ' + json.dumps(answer),
                                         model='gpt-6-astra', outputSchema={'title': 'review'}), answer)

    def test_claude_stages_model(self):
        p = self.claude()
        p.send({'type': 'user', 'message': {'role': 'user', 'content': 'first'}})
        session = p.receive(lambda e: e.get('type') == 'system')['session_id']
        first = p.receive(lambda e: e.get('type') == 'result')
        self.assertEqual(list(first['modelUsage']), ['m1'])
        self.assertGreater(first['modelUsage']['m1']['inputTokens'], 0)
        self.assertGreater(first['modelUsage']['m1']['outputTokens'], 0)
        p.close()
        p2 = self.claude('m2', resume=session)
        p2.send({'type': 'user', 'message': {'role': 'user', 'content': 'second'}})
        self.assertEqual(p2.receive(lambda e: e.get('type') == 'system')['session_id'], session)
        self.assertEqual(list(p2.receive(lambda e: e.get('type') == 'result')['modelUsage']), ['m2'])
        record = json.loads((self.root / 'record.json').read_text(encoding='utf-8'))
        self.assertEqual(record['turn_models'], ['m1', 'm2'])

    def test_set_model_supported(self):
        p = self.claude(env={'FAKE_CLAUDE_SET_MODEL': 'supported'})
        self.assertEqual(list(self.user(p)['modelUsage']), ['m1'])
        response, request = self.control(p, 'set_model')
        self.assertEqual(response['response']['subtype'], 'success')
        self.assertEqual(response['response']['request_id'], 'req-1')
        self.assertEqual(list(self.user(p)['modelUsage']), ['m2'])
        record = json.loads((self.root / 'record.json').read_text(encoding='utf-8'))
        self.assertIn(request, record['control_requests'])

    def test_set_model_refused(self):
        p = self.claude()
        self.user(p)
        response, request = self.control(p, 'set_model')
        self.assertEqual(response['response']['subtype'], 'error')
        self.assertEqual(response['response']['request_id'], 'req-1')
        self.assertEqual(list(self.user(p)['modelUsage']), ['m1'])
        self.assertIn(request, json.loads((self.root / 'record.json').read_text(encoding='utf-8'))['control_requests'])

    def test_other_subtype_refused(self):
        p = self.claude(env={'FAKE_CLAUDE_SET_MODEL': 'supported'})
        self.user(p)
        response, request = self.control(p, 'unknown')
        self.assertEqual(response['response']['subtype'], 'error')
        self.assertEqual(response['response']['request_id'], 'req-1')
        self.assertEqual(list(self.user(p)['modelUsage']), ['m1'])
        self.assertIn(request, json.loads((self.root / 'record.json').read_text(encoding='utf-8'))['control_requests'])

    def test_defaults_unchanged(self):
        p = self.process(CODEX)
        p.send({'id': 1, 'method': 'model/list', 'params': {}})
        result = p.receive(lambda e: e.get('id') == 1)['result']
        self.assertEqual(result['data'][0]['id'], 'gpt-6-luna')
        self.assertEqual(result['nextCursor'], 'page-2')
        p.send({'id': 3, 'method': 'model/list', 'params': {'cursor': 'page-2'}})
        self.assertEqual(p.receive(lambda e: e.get('id') == 3)['result']['data'][0]['id'], 'gpt-6.10-terra')
        p.send({'id': 2, 'method': 'thread/start', 'params': {'model': 'm'}})
        p.receive(lambda e: e.get('id') == 2)
        answer = self.answer_turn(p, 'ordinary\nFAKE_WRITE legacy.txt')
        self.assertEqual(answer['model'], 'm')
        self.assertEqual(answer['brief'], 'ordinary\nFAKE_WRITE legacy.txt')
        self.assertEqual((self.root / 'legacy.txt').read_text(encoding='utf-8'), 'fake-written\n')
        c = self.process(CLAUDE, {'FAKE_CLAUDE_RECORD': self.root / 'default.json'})
        result = self.user(c)
        self.assertEqual(result['result'], '統計功能已完成。')
        self.assertIn('claude-opus-5-5', result['modelUsage'])


if __name__ == '__main__':
    unittest.main(verbosity=2)
