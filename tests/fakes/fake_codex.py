"""Scripted stand-in for `codex app-server`; the brief text selects the scenario."""
import json
import os
import sys
import time
import threading

sys.stdin.reconfigure(encoding='utf-8')
sys.stdout.reconfigure(encoding='utf-8')
if '--version' in sys.argv:
    print('codex-cli fake')
    sys.exit(0)

pending_turns = {}
send_lock = threading.Lock()


def send(message):
    with send_lock:
        sys.stdout.write(json.dumps(message, ensure_ascii=False) + '\n')
        sys.stdout.flush()


def item(thread_id, turn_id, payload):
    send({'method': 'item/completed', 'params': {'threadId': thread_id, 'turnId': turn_id, 'completedAtMs': 0, 'item': payload}})


def complete(thread_id, turn_id, status='completed', error=None):
    send({'method': 'turn/completed', 'params': {'threadId': thread_id, 'turn': {'id': turn_id, 'status': status, 'error': error, 'items': []}}})


threads = 0
models = {}
configs = {}
thread_params = {}
for line in sys.stdin:
    message = json.loads(line)
    method, params = message.get('method'), message.get('params') or {}
    if method and os.getenv('FAKE_CODEX_LOG'):
        with open(os.environ['FAKE_CODEX_LOG'], 'a', encoding='utf-8') as log:
            log.write(json.dumps({'method': method, 'params': params, 'time': time.monotonic()}) + '\n')
    if method is None:
        if 'error' in message:
            thread_id, turn_id = pending_turns.pop('approval')
            item(thread_id, turn_id, {'type': 'agentMessage', 'id': 'm', 'text': 'approval refused'})
            complete(thread_id, turn_id)
        continue
    if method == 'initialize':
        send({'id': message['id'], 'result': {'userAgent': 'fake'}})
    elif method == 'model/list':
        if params.get('cursor'):
            send({'id': message['id'], 'result': {'data': [{'id': 'gpt-6.10-terra'}, {'id': 'gpt-6-astra'}, {'id': 'gpt-6-terra-lite'}]}})
        else:
            send({'id': message['id'], 'result': {'data': [{'id': 'gpt-6-luna', 'isDefault': True}, {'id': 'gpt-5.6-terra'}, {'model': 'gpt-6.9-terra'}], 'nextCursor': 'page-2'}})
    elif method == 'plugin/installed':
        send({'id': message['id'], 'result': {'marketplaces': [{'name': 'personal', 'plugins': [
            {'id': 'panel@personal', 'enabled': True}, {'id': 'keepme@personal', 'enabled': True}, {'id': 'off@personal', 'enabled': False}]}]}})
    elif method == 'config/read':
        send({'id': message['id'], 'result': {'config': {'mcp_servers': {'node_repl': {'command': 'x'}}}, 'origins': {}}})
    elif method == 'thread/start':
        threads += 1
        thread_id = f'thread-{threads}'
        models[thread_id] = params.get('model')
        config = params.get('config') or {}
        configs[thread_id] = config
        thread_params[thread_id] = params
        send({'id': message['id'], 'result': {'thread': {'id': thread_id}}})
        send({'method': 'thread/started', 'params': {'thread': {'id': thread_id}}})
        servers = {'node_repl': ('mcp_servers', 'node_repl'), 'panel': ('plugins', 'panel@personal'), 'keepme': ('plugins', 'keepme@personal')}
        for server, (section, key) in servers.items():
            if (config.get(section) or {}).get(key, {}).get('enabled') is not False:
                send({'method': 'mcpServer/startupStatus/updated', 'params': {'threadId': thread_id, 'name': server, 'status': 'ready'}})
        if (config.get('features') or {}).get('apps') is not False:
            send({'method': 'mcpServer/startupStatus/updated', 'params': {'threadId': thread_id, 'name': 'codex_apps', 'status': 'ready'}})
    elif method == 'turn/start':
        thread_id, text = params['threadId'], params['input'][0]['text']
        turn_id = thread_id + '-turn'
        if 'HANGREQ' in text:
            continue
        send({'id': message['id'], 'result': {'turn': {'id': turn_id, 'status': 'inProgress'}}})
        if 'CRASH' in text:
            sys.exit(7)
        if 'HANG' in text:
            pending_turns[turn_id] = (thread_id, turn_id)
            continue
        if 'SLOW' in text:
            pending_turns[turn_id] = (thread_id, turn_id)
            def finish_slow(tid=thread_id, turn=turn_id):
                time.sleep(2)
                if pending_turns.pop(turn, None):
                    item(tid, turn, {'type': 'agentMessage', 'id': 'm', 'text': 'slow'})
                    complete(tid, turn)
            threading.Thread(target=finish_slow, daemon=True).start()
            continue
        if 'APPROVAL' in text:
            pending_turns['approval'] = (thread_id, turn_id)
            send({'id': 'srv-1', 'method': 'item/commandExecution/requestApproval', 'params': {'threadId': thread_id, 'turnId': turn_id}})
            continue
        if 'BADMODEL' in text:
            body = json.dumps({'type': 'error', 'status': 400, 'error': {'type': 'invalid_request_error', 'message': 'not supported', 'code': 'model_not_found'}})
            complete(thread_id, turn_id, 'failed', {'message': body, 'codexErrorInfo': 'other'})
            continue
        if 'AUTH' in text:
            complete(thread_id, turn_id, 'failed', {'message': 'auth', 'codexErrorInfo': 'AuthError'})
            continue
        if 'LIMIT' in text:
            error = {'message': 'You have hit your usage limit.', 'codexErrorInfo': 'usageLimitExceeded'}
            send({'method': 'error', 'params': {'threadId': thread_id, 'turnId': turn_id, 'willRetry': False, 'error': error}})
            complete(thread_id, turn_id, 'failed', error)
            continue
        item(thread_id, turn_id, {'type': 'commandExecution', 'id': 'c', 'command': 'pytest -q', 'status': 'completed', 'exitCode': 0})
        item(thread_id, turn_id, {'type': 'fileChange', 'id': 'f', 'status': 'completed',
                                  'changes': [{'path': 'src/mod.py', 'kind': {'type': 'add'}, 'diff': ''}]})
        usage = {'inputTokens': 1200, 'cachedInputTokens': 800, 'outputTokens': 40, 'reasoningOutputTokens': 10, 'totalTokens': 1240, 'cacheWriteInputTokens': 0}
        send({'method': 'thread/tokenUsage/updated', 'params': {'threadId': thread_id, 'turnId': turn_id, 'tokenUsage': {'total': usage, 'last': usage}}})
        echo = {'model': models[thread_id], 'effort': params.get('effort'), 'brief': text, 'config': configs[thread_id],
                'disabled_plugin_ids': params.get('disabledPluginIds'), 'thread_params': thread_params[thread_id]}
        final = json.dumps({'answer': 'ok'}) if params.get('outputSchema') else json.dumps(echo, ensure_ascii=False)
        if params.get('outputSchema') and 'BADJSON' in text:
            final = 'not json'
        elif params.get('outputSchema') and 'WRONGSHAPE' in text:
            final = json.dumps({'wrong': 'shape'})
        item(thread_id, turn_id, {'type': 'agentMessage', 'id': 'm', 'text': final})
        complete(thread_id, turn_id)
    elif method == 'turn/interrupt':
        send({'id': message['id'], 'result': {}})
        turn = pending_turns.pop(params['turnId'], None)
        if turn:
            complete(*turn, 'interrupted')
    time.sleep(0)
