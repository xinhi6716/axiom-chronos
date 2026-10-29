"""Run the goal hooks the way Claude Code does: one JSON event on stdin, the decision on stdout."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ENGINE = Path(__file__).resolve().parent.parent / 'src' / 'axiom' / 'engine'
sys.path.insert(0, str(ENGINE / 'bridge'))

from procs import BridgeError  # noqa: E402

SESSION = '6a489068-399b-48fa-9405-a1a033086576'
CODEX = 'mcp__plugin_chronos_bridge__codex_run'


class GoalHookTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.env = {**os.environ, 'AXIOM_DATA_HOME': self.temp.name}
        self.env.pop('AXIOM_HEADLESS', None)
        # Hooks must not depend on UTF-8 mode: CI's Windows console is cp1252.
        self.env.update(PYTHONUTF8='0', PYTHONIOENCODING='cp1252')

    def hook(self, event, **extra):
        proc = subprocess.run([sys.executable, str(ENGINE / 'hooks' / 'goal.py')], input=json.dumps({'session_id': SESSION, **event}),
                              capture_output=True, text=True, encoding='utf-8', env={**self.env, **extra}, check=True)
        return json.loads(proc.stdout) if proc.stdout.strip() else None

    def prompt(self, text, **extra):
        return self.hook({'hook_event_name': 'UserPromptSubmit', 'prompt': text, 'cwd': self.temp.name}, **extra)

    def stop(self):
        return self.hook({'hook_event_name': 'Stop', 'stop_hook_active': False})

    def close(self, status, note=''):
        import goal
        with patch.dict(os.environ, {'AXIOM_DATA_HOME': self.temp.name}):
            return goal.set_status(SESSION, status, note)

    def test_wish_holds_the_session_until_done(self):
        self.assertIsNone(self.stop())
        context = self.prompt('/chronos:wish 加一個 --stats 參數')['hookSpecificOutput']['additionalContext']
        self.assertIn(SESSION, context)
        self.assertNotIn('headless', context)
        self.assertEqual(self.stop()['decision'], 'block')
        self.assertEqual(self.close('done')['status'], 'completed')
        self.assertIsNone(self.stop())

    def test_later_messages_reopen_the_goal(self):
        self.prompt('/chronos:wish 修 parser')
        self.close('done')
        self.assertIn('amends', self.prompt('再加一個測試')['hookSpecificOutput']['additionalContext'])
        self.assertEqual(self.stop()['decision'], 'block')

    def test_question_follow_up_keeps_active_goal_blocked(self):
        self.prompt('/chronos:wish 修 parser')
        context = self.prompt('目前進度如何？')['hookSpecificOutput']['additionalContext']
        self.assertNotIn('answer it', context)
        self.assertEqual(self.stop()['decision'], 'block')

    def test_task_notifications_do_not_reopen_the_goal(self):
        self.prompt('/chronos:wish 修 parser')
        self.close('done')
        self.assertIsNone(self.prompt('<task-notification>\n<status>completed</status>\n</task-notification>'))
        self.assertIsNone(self.stop())

    def test_ordinary_sessions_are_left_alone(self):
        self.assertIsNone(self.prompt('/chronos:research RRAM'))
        self.assertIsNone(self.stop())

    def test_blocked_needs_a_reason(self):
        self.prompt('/chronos:wish 部署')
        with self.assertRaises(BridgeError):
            self.close('blocked')
        self.close('blocked', 'needs the deploy key')
        self.assertIsNone(self.stop())

    def test_headless_runs_are_told_to_ask_nothing(self):
        context = self.prompt('/chronos:wish x', AXIOM_HEADLESS='1')['hookSpecificOutput']['additionalContext']
        self.assertIn('headless', context)

    def test_only_agents_call_codex(self):
        event = {'hook_event_name': 'PreToolUse', 'tool_name': CODEX, 'tool_input': {}}
        self.assertEqual(self.hook(event)['hookSpecificOutput']['permissionDecision'], 'deny')
        self.assertIsNone(self.hook({**event, 'agent_id': 'a1', 'agent_type': 'chronos:dispatcher'}))



if __name__ == '__main__':
    unittest.main()
