"""Stand-in for `claude -p ... --output-format stream-json`: records its launch, then plays a scenario."""
import json
import os
import sys

sys.stdout.reconfigure(encoding='utf-8')
if '--version' in sys.argv:
    print('2.1.281 (Claude Code)')
    sys.exit(0)
with open(os.environ['FAKE_CLAUDE_RECORD'], 'w', encoding='utf-8') as record:
    json.dump({'argv': sys.argv[1:], 'cwd': os.getcwd(),
               'env': {k: os.environ.get(k) for k in ('AXIOM_DATA_HOME', 'AXIOM_PYTHON')}}, record)
scenario = os.environ.get('FAKE_CLAUDE_SCENARIO', 'ok')


def emit(event):
    sys.stdout.write(json.dumps(event, ensure_ascii=False) + '\n')


bridge_status = 'failed' if scenario == 'no-bridge' else 'connected'
emit({'type': 'system', 'subtype': 'init', 'mcp_servers': [{'name': 'plugin:chronos:bridge', 'status': bridge_status}]})
if scenario == 'crash':
    sys.exit(3)
emit({'type': 'assistant', 'parent_tool_use_id': None, 'message': {'content': [
    {'type': 'tool_use', 'id': 'a1', 'name': 'Agent', 'input': {'subagent_type': 'chronos:dispatcher', 'description': 'Dispatch wish'}}]}})
emit({'type': 'assistant', 'parent_tool_use_id': 'a1', 'message': {'content': [
    {'type': 'tool_use', 'id': 't1', 'name': 'mcp__plugin_chronos_bridge__orch', 'input': {'project': 'p', 'args': ['unit', 'add', 'u1']}},
    {'type': 'tool_use', 'id': 't2', 'name': 'mcp__plugin_chronos_bridge__codex_run',
     'input': {'role': 'terra', 'brief': 'GOAL    todo.py defines stats(items).\nSCOPE   todo.py\nVERIFY  python -m unittest'}},
    {'type': 'tool_use', 'id': 't3', 'name': 'Read', 'input': {'file_path': 'x'}}]}})
emit({'type': 'user', 'parent_tool_use_id': None, 'message': {'content': [{'type': 'tool_result', 'tool_use_id': 'a1', 'content': 'done'}]}})
emit({'type': 'result', 'subtype': 'success', 'is_error': scenario == 'error', 'result': '統計功能已完成。',
      'duration_ms': 4200, 'total_cost_usd': 0.36, 'modelUsage': {'claude-opus-5-5': {'costUSD': 0.25}}})
