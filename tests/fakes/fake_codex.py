"""Scripted stand-in for `codex app-server`.

The brief text selects the turn scenario. Command-line options reshape the CLI itself:
--fake-version=<line> is what `--version` prints, --fake-version-exit=<n> sets its exit code,
--fake-version-hang never answers, --fake-version-holder=<pid file> answers and exits but leaves a
child holding its output streams (as a wrapper whose child outlives it), writing that child's pid
to the file, and --fake-break=<method>:<effect> makes that request misbehave
with exit, silent, not-json, no-result, empty-result or <code>:<message>. Method `start` exits
before reading anything, as when the `app-server` command line is rejected.
"""
import json
import os
import re
import sys
import time
import threading

sys.stdin.reconfigure(encoding='utf-8')
sys.stdout.reconfigure(encoding='utf-8')


def option(name, default=''):
    prefix = f'--fake-{name}='
    return next((arg[len(prefix):] for arg in sys.argv if arg.startswith(prefix)), default)


if '--version' in sys.argv:
    if '--fake-version-hang' in sys.argv:
        time.sleep(60)
    print(option('version', 'codex-cli fake'), flush=True)
    if option('version-holder'):
        import subprocess
        holder = subprocess.Popen([sys.executable, '-I', '-c', 'import time; time.sleep(30)'],
                                  stdin=subprocess.DEVNULL, close_fds=False)
        with open(option('version-holder'), 'w') as f:
            f.write(str(holder.pid))
    sys.exit(int(option('version-exit', '0')))
broken_method, _, breakage = option('break').partition(':')
if broken_method == 'start':
    sys.exit(2)

pending_turns = {}
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
    if method and method == broken_method:
        if breakage == 'exit':
            sys.exit(2)
        if breakage == 'no-result':
            send({'id': message['id']})
        elif breakage == 'empty-result':
            send({'id': message['id'], 'result': {}})
        elif breakage == 'not-json':
            sys.stdout.write('Content-Length: 2\n')
            sys.stdout.flush()
        elif breakage != 'silent':
            code, _, text = breakage.partition(':')
            send({'id': message['id'], 'error': {'code': int(code), 'message': text}})
        continue
    if method is None:
        if 'error' in message:
            thread_id, turn_id = pending_turns.pop('approval')
            item(thread_id, turn_id, {'type': 'agentMessage', 'id': 'm', 'text': 'approval refused'})
            complete(thread_id, turn_id)
        continue
    if method == 'initialize':
        send({'id': message['id'], 'result': {'userAgent': 'fake'}})
    elif method == 'account/rateLimits/read':
        limits = {'limitId': 'codex',
                  'primary': {'usedPercent': 38, 'windowDurationMins': 300, 'resetsAt': 1900000000},
                  'secondary': {'usedPercent': 59, 'windowDurationMins': 10080, 'resetsAt': 1900500000}}
        send({'id': message['id'], 'result': {'rateLimits': limits, 'rateLimitsByLimitId': {'codex': limits}}})
        if os.getenv('FAKE_CODEX_ACCOUNT_UPDATE'):
            send({'method': 'thread/started', 'params': {}})
            if os.getenv('FAKE_CODEX_ACCOUNT_UPDATE') == 'two':
                send({'method': 'account/other', 'params': {}})
            send({'method': 'account/rateLimits/updated', 'params': {'rateLimits': {'primary': {'usedPercent': 41, 'windowDurationMins': 300, 'resetsAt': 1900000000}}}})
    elif method == 'model/list':
        if params.get('cursor'):
            send({'id': message['id'], 'result': {'data': [{'id': 'gpt-6.10-terra'}, {'id': 'gpt-6-astra'}, {'id': 'gpt-6-terra-lite'}]}})
        else:
            send({'id': message['id'], 'result': {'data': [{'id': 'gpt-6-luna', 'isDefault': True}, {'id': 'gpt-5.6-terra'}, {'model': 'gpt-6.9-terra'}], 'nextCursor': 'page-2'}})
    elif method == 'plugin/installed':
        send({'id': message['id'], 'result': {'marketplaces': [{'name': 'personal', 'plugins': [
            {'id': 'panel@personal', 'enabled': True}, {'id': 'keepme@personal', 'enabled': True}, {'id': 'off@personal', 'enabled': False}]}]}})
    elif method == 'config/read':
        servers = {'node_repl': {'command': 'x'}}
        if os.getenv('FAKE_CODEX_CODEGRAPH'):
            servers['codegraph'] = {'command': 'codegraph'}
        send({'id': message['id'], 'result': {'config': {'mcp_servers': servers}, 'origins': {}}})
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
        if 'BARRIER' in text:
            with barrier:
                barrier_count += 1
                barrier.notify_all()
            def finish_barrier(tid=thread_id, turn=turn_id, extra='EXTRA' in text):
                with barrier:
                    ready = barrier.wait_for(lambda: barrier_count >= 2, timeout=10)
                if ready:
                    if extra:
                        send({'method': 'test/other-turn', 'params': {'threadId': tid, 'turnId': 'other-turn'}})
                        send({'method': 'mcpServer/startupStatus/updated', 'params': {'threadId': tid, 'status': 'starting'}})
                        send({'method': 'test/extra', 'params': {'threadId': tid, 'turnId': turn}})
                    item(tid, turn, {'type': 'agentMessage', 'id': 'm', 'text': 'barrier'})
                    complete(tid, turn)
                else:
                    complete(tid, turn, 'failed', {'message': 'barrier timeout'})
            threading.Thread(target=finish_barrier, daemon=True).start()
            continue
        if 'WAIT_FOR_INTERRUPT' in text:
            pending_turns[turn_id] = (thread_id, turn_id)
            def expire_wait(tid=thread_id, turn=turn_id):
                time.sleep(60)
                if pending_turns.pop(turn, None):
                    complete(tid, turn, 'failed', {'message': 'interrupt timeout'})
            threading.Thread(target=expire_wait, daemon=True).start()
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
        if 'ODDSTATUS' in text:
            complete(thread_id, turn_id, 'succeeded')
            continue
        if 'NOSTATUS' in text:
            complete(thread_id, turn_id, None)
            continue
        if 'ODDFAILURE' in text:
            complete(thread_id, turn_id, 'cancelled', {'message': 'kept', 'codexErrorInfo': 'other'})
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
        if params.get('outputSchema') and 'CROSS-EXAMINATION' in text:
            ids = sorted(set(re.findall(r'^(C\d+) \[Position', text, flags=re.M)))
            final = json.dumps({'stances': [{'claim': i, 'stance': 'agree', 'evidence': ''} for i in ids]})
        if params.get('outputSchema') and 'DIFF REVIEW' in text:
            final = json.dumps({'verdict': 'revise', 'findings': [{'severity': 'major', 'location': 'src/a.py:1', 'reason': 'codex reviewed this diff'}]}) if 'BLOCKME' in text else json.dumps({'verdict': 'pass', 'findings': []})
        elif params.get('outputSchema') and 'BADJSON' in text:
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
