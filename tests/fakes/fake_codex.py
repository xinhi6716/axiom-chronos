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
dynamic_calls = {}
dynamic_results = {}
send_lock = threading.Lock()
log_lock = threading.Lock()
barrier = threading.Condition()
barrier_count = 0


def log_record(method, params):
    path = os.getenv('FAKE_CODEX_LOG')
    if not path:
        return
    with log_lock:
        deadline = time.monotonic() + 10
        while True:
            try:
                fd = os.open(path + '.lock', os.O_CREAT | os.O_EXCL | os.O_WRONLY)
                break
            except (FileExistsError, PermissionError):
                if time.monotonic() >= deadline:
                    raise TimeoutError('fake codex log lock timeout')
                time.sleep(0.01)
        try:
            with open(path, 'a', encoding='utf-8') as log:
                log.write(json.dumps({'method': method, 'params': params, 'time': time.monotonic()}) + '\n')
        finally:
            os.close(fd)
            os.unlink(path + '.lock')


def send(message):
    with send_lock:
        sys.stdout.write(json.dumps(message, ensure_ascii=False) + '\n')
        sys.stdout.flush()


def item(thread_id, turn_id, payload):
    send({'method': 'item/completed', 'params': {'threadId': thread_id, 'turnId': turn_id, 'completedAtMs': 0, 'item': payload}})


def complete(thread_id, turn_id, status='completed', error=None):
    log_record('fake/complete', {'threadId': thread_id, 'turnId': turn_id, 'status': status})
    send({'method': 'turn/completed', 'params': {'threadId': thread_id, 'turn': {'id': turn_id, 'status': status, 'error': error, 'items': []}}})


threads = 0
models = {}
configs = {}
thread_params = {}
for line in sys.stdin:
    message = json.loads(line)
    method, params = message.get('method'), message.get('params') or {}
    if method:
        log_record(method, params)
    if method is None:
        if message.get('id') in dynamic_calls:
            thread_id, turn_id = dynamic_calls.pop(message['id'])
            log_record('fake/tool_response', message)
            result = message.get('result', {})
            dynamic_results.setdefault(thread_id, []).append(result)
            if not any(tid == thread_id for tid, _ in dynamic_calls.values()):
                valid = all(r.get('success') is True and r.get('contentItems', [{}])[0].get('type') == 'inputText' for r in dynamic_results.pop(thread_id))
                item(thread_id, turn_id, {'type': 'agentMessage', 'id': 'm', 'text': 'dynamic complete'})
                complete(thread_id, turn_id, 'completed' if valid else 'failed')
            continue
        if 'error' in message:
            thread_id, turn_id = pending_turns.pop('approval')
            item(thread_id, turn_id, {'type': 'agentMessage', 'id': 'm', 'text': 'approval refused'})
            complete(thread_id, turn_id)
        continue
    if method == 'initialize':
        assert params.get('capabilities', {}).get('experimentalApi') is True
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
    elif method == 'skills/list':
        send({'id': message['id'], 'result': {'data': [{'cwd': params['cwds'][0], 'errors': [], 'skills': [
            {'name': 'ordinary', 'path': '/skills/ordinary/SKILL.md', 'enabled': True, 'scope': 'user', 'description': ''},
            {'name': 'workflow', 'path': '/skills/workflow/SKILL.md', 'pluginId': 'keepme@personal', 'enabled': True, 'scope': 'user', 'description': ''}]}]}})
    elif method == 'thread/start':
        threads += 1
        thread_id = f'thread-{threads}'
        models[thread_id] = 'actual-model' if params.get('model') == 'requested-model' else params.get('model')
        config = params.get('config') or {}
        configs[thread_id] = config
        thread_params[thread_id] = params
        mode = params.get('developerInstructions', '')
        actual = 'gpt-6-sol' if mode == 'FAKE_WRONG_MODEL' else models[thread_id]
        sandbox = 'workspaceWrite' if mode == 'FAKE_WRONG_SANDBOX' else ('readOnly' if params.get('sandbox') == 'read-only' else 'workspaceWrite')
        approval = 'on-request' if mode == 'FAKE_WRONG_APPROVAL' else 'never'
        send({'id': message['id'], 'result': {'thread': {'id': thread_id}, 'model': actual, 'sandbox': {'type': sandbox}, 'approvalPolicy': approval}})
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
        if 'NO_TURN_ID' in text:
            send({'id':message['id'],'result':{'turn':{}}})
            continue
        send({'id': message['id'], 'result': {'turn': {'id': turn_id, 'status': 'inProgress'}}})
        if 'CRASH' in text:
            sys.exit(7)
        if 'REFUSE_INTERRUPT' in text:
            pending_turns[turn_id] = (thread_id, turn_id, 'refuse')
            continue
        if 'NO_CONFIRM' in text:
            pending_turns[turn_id] = (thread_id, turn_id, 'no_confirm')
            continue
        if 'LATE_COMPLETE' in text:
            pending_turns[turn_id] = (thread_id, turn_id, 'late_complete')
            continue
        if 'HANG' in text:
            pending_turns[turn_id] = (thread_id, turn_id)
            continue
        if 'BARRIER' in text:
            with barrier:
                barrier_count += 1
                barrier.notify_all()
            def finish_barrier(tid=thread_id, turn=turn_id):
                with barrier:
                    ready = barrier.wait_for(lambda: barrier_count >= 2, timeout=10)
                if ready:
                    item(tid, turn, {'type': 'agentMessage', 'id': 'm', 'text': 'barrier'})
                    complete(tid, turn)
                else:
                    complete(tid, turn, 'failed', {'message': 'barrier timeout'})
            threading.Thread(target=finish_barrier, daemon=True).start()
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
        if 'DYNAMIC' in text:
            for n in range(2):
                call_id = f'dynamic-{thread_id}-{n}'
                dynamic_calls[call_id] = (thread_id, turn_id)
                send({'id': call_id, 'method': 'item/tool/call', 'params': {'threadId': thread_id, 'turnId': turn_id, 'callId': call_id, 'tool': 'test', 'arguments': {'n': n, 'threadId': thread_id}}})
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
        if text.startswith('EFFORT_'):
            scenarios = {
                'EFFORT_NESTED': {'threadSettings': {'effort': 'high'}, 'effort': 'legacy'},
                'EFFORT_MISSING': {'threadSettings': {}},
                'EFFORT_NULL': {'threadSettings': {'effort': None}, 'effort': 'legacy'},
                'EFFORT_INVALID': {'threadSettings': {'effort': 42}, 'effort': 'legacy'},
                'EFFORT_EMPTY': {'threadSettings': {'effort': ''}, 'effort': 'legacy'},
                'EFFORT_LEGACY': {'threadSettings': {}, 'effort': 'medium'},
                'EFFORT_ROUTING': {'threadSettings': {}},
            }
            if text == 'EFFORT_ROUTING':
                send({'method': 'thread/settings/updated', 'params': {'threadId': 'other-thread', 'threadSettings': {'effort': 'wrong-thread'}}})
                send({'method': 'thread/settings/updated', 'params': {'threadId': thread_id, 'turnId': 'other-turn', 'threadSettings': {'effort': 'wrong-turn'}}})
            send({'method': 'thread/settings/updated', 'params': {'threadId': thread_id, **scenarios[text]}})
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
        turn = pending_turns.get(params['turnId'])
        if turn and len(turn) == 3 and turn[2] == 'refuse':
            send({'id': message['id'], 'error': {'code': -32000, 'message': 'refused'}})
            continue
        send({'id': message['id'], 'result': {}})
        turn = pending_turns.pop(params['turnId'], None)
        if turn and len(turn) == 2:
            complete(*turn, 'interrupted')
        elif turn and turn[2] == 'late_complete':
            complete(*turn[:2], 'completed')
    time.sleep(0)
