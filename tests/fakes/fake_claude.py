"""Scripted stream-json Claude stand-in."""
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import threading

sys.stdin.reconfigure(encoding='utf-8')
sys.stdout.reconfigure(encoding='utf-8')
if '--version' in sys.argv:
    print('2.1.281 (Claude Code)')
    sys.exit(0)
scenario = os.environ.get('FAKE_CLAUDE_SCENARIO', 'ok')
session = os.environ.get('FAKE_CLAUDE_SESSION', '12345678-1234-1234-1234-123456789abc')
record = {'argv': sys.argv[1:], 'cwd': os.getcwd(), 'env': {k: os.environ.get(k) for k in ('AXIOM_DATA_HOME', 'AXIOM_PYTHON', 'AXIOM_HEADLESS', 'AXIOM_BIN')}, 'messages': []}

def save():
    with open(os.environ['FAKE_CLAUDE_RECORD'], 'w', encoding='utf-8') as f:
        json.dump(record, f, ensure_ascii=False)

def emit(event):
    print(json.dumps(event, ensure_ascii=False), flush=True)

incoming = queue.Queue()
def read_input():
    for line in sys.stdin:
        try:
            message = json.loads(line)
        except ValueError:
            continue
        record['messages'].append(message)
        save()
        incoming.put(message)
    incoming.put(None)
threading.Thread(target=read_input, daemon=True).start()
save()
first = incoming.get()
if first is None:
    sys.exit(0)
emit({'type': 'system', 'subtype': 'init', 'session_id': session, 'mcp_servers': [{'name': 'plugin:chronos:bridge', 'status': 'failed' if scenario == 'no-bridge' else 'connected'}]})
if scenario == 'limit':
    emit({'type': 'result', 'is_error': True, 'result': "You've hit your usage limit"})
    sys.exit(0)
if scenario == 'resume':
    path = Path(os.environ['AXIOM_DATA_HOME']) / 'goals' / (session + '.json')
    goal = json.loads(path.read_text(encoding='utf-8'))
    goal['status'] = 'done'
    path.write_text(json.dumps(goal, ensure_ascii=False) + '\n', encoding='utf-8')
    emit({'type': 'result', 'is_error': False, 'result': 'done'})
    sys.exit(0)
if scenario == 'crash':
    sys.exit(3)
if scenario == 'interrupt':
    child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])
    Path(os.environ['FAKE_CLAUDE_PIDS']).write_text(f'{os.getpid()} {child.pid}', encoding='utf-8')
    child.wait()
    sys.exit(0)
if scenario == 'forward':
    for _ in range(2):
        if incoming.get(timeout=5) is None:
            sys.exit(5)
    emit({'type': 'result', 'is_error': False, 'result': 'forwarded'})
    sys.exit(0)
if scenario == 'interject':
    emit({'type': 'assistant', 'message': {'content': [{'type': 'tool_use', 'id': 't2', 'name': 'mcp__plugin_chronos_bridge__codex_run', 'input': {'role': 'terra', 'brief': 'GOAL wait'}}]}})
    try:
        message = incoming.get(timeout=5)
    except queue.Empty:
        sys.exit(5)
    if message is None:
        sys.exit(5)
    text = message.get('message', {}).get('content', [{}])[0].get('text', '')
    emit({'type': 'user', 'message': {'content': [{'type': 'tool_result', 'tool_use_id': 't2', 'content': text}]}})
    emit({'type': 'result', 'is_error': False, 'result': text, 'duration_ms': 1000, 'total_cost_usd': 0.3})
    sys.exit(0)
if scenario == 'continue':
    emit({'type': 'result', 'is_error': False, 'result': 'continuing', 'duration_ms': 1000, 'total_cost_usd': 0.30})
    if incoming.get() is None:
        sys.exit(5)
    path = Path(os.environ['AXIOM_DATA_HOME']) / 'goals' / (session + '.json')
    goal = json.loads(path.read_text(encoding='utf-8'))
    goal['status'] = 'done'
    path.write_text(json.dumps(goal, ensure_ascii=False) + '\n', encoding='utf-8')
    emit({'type': 'result', 'is_error': False, 'result': 'done', 'duration_ms': 2000, 'total_cost_usd': 0.50})
    while incoming.get() is not None:
        pass
    sys.exit(0)
emit({'type': 'assistant', 'parent_tool_use_id': None, 'message': {'content': [{'type': 'tool_use', 'id': 'a1', 'name': 'Agent', 'input': {'subagent_type': 'chronos:dispatcher', 'description': 'Dispatch wish'}}]}})
emit({'type': 'assistant', 'parent_tool_use_id': 'a1', 'message': {'content': [{'type': 'tool_use', 'id': 't1', 'name': 'mcp__plugin_chronos_bridge__orch', 'input': {'args': ['unit', 'add', 'u1']}}, {'type': 'tool_use', 'id': 't2', 'name': 'mcp__plugin_chronos_bridge__codex_run', 'input': {'role': 'terra', 'brief': 'GOAL    todo.py defines stats(items).\nSCOPE   todo.py'}}, {'type': 'tool_use', 'id': 't3', 'name': 'Read', 'input': {}}]}})
emit({'type': 'user', 'message': {'content': [{'type': 'tool_result', 'tool_use_id': 'a1', 'content': 'done'}]}})
emit({'type': 'result', 'subtype': 'success', 'is_error': scenario == 'error', 'result': '統計功能已完成。', 'duration_ms': 4200, 'total_cost_usd': 0.36, 'modelUsage': {'claude-opus-5-5': {'costUSD': 0.25}}})
