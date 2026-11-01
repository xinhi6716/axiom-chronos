"""RPC regression fixture, not live acceptance or a production execution path."""
import json
import os
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.stdin.reconfigure(encoding='utf-8')
sys.stdout.reconfigure(encoding='utf-8')
log_path = Path(sys.argv[1])
mode = sys.argv[2] if len(sys.argv) > 2 else 'normal'
lock = threading.Lock()
threads = {}
pending = {}
barrier = threading.Barrier(2, timeout=10)


def send(value):
    with lock:
        print(json.dumps(value), flush=True)


def log(kind, **values):
    with lock:
        with log_path.open('a', encoding='utf-8') as out:
            out.write(json.dumps(dict(kind=kind, time=time.monotonic(), **values)) + '\n')


def finish(tid, text='done', failed=False):
    turn = tid + '-turn'
    send({'method': 'item/completed', 'params': {'threadId': tid, 'turnId': turn,
          'item': {'type': 'agentMessage', 'id': 'message', 'text': text}}})
    send({'method': 'turn/completed', 'params': {'threadId': tid,
          'turn': {'id': turn, 'status': 'failed' if failed else 'completed'}}})


def call(tid, name, args, final='done'):
    cid = 'tool-' + tid
    pending[cid] = (tid, final)
    send({'id': cid, 'method': 'item/tool/call', 'params': {
        'threadId': tid, 'turnId': tid + '-turn', 'callId': cid,
        'tool': name, 'arguments': args}})


for line in sys.stdin:
    msg = json.loads(line)
    method, p = msg.get('method'), msg.get('params', {})
    if not method:
        if msg.get('id') in pending:
            tid, final = pending.pop(msg['id'])
            log('reply', thread=tid, response=msg)
            finish(tid, final, 'error' in msg or not msg.get('result', {}).get('success'))
        continue
    if method == 'initialize':
        result = {}
    elif method == 'model/list':
        result = {'data': [{'id': 'gpt-6.1-' + role,
                  'supportedReasoningEfforts': [{'reasoningEffort': e} for e in ['medium', 'high', 'xhigh', 'max']]}
                  for role in ['terra', 'sol', 'astra', 'luna']]}
    elif method == 'plugin/installed':
        result = {'marketplaces': []}
    elif method == 'config/read':
        result = {'config': {}}
    elif method == 'thread/start':
        tid = 't-' + str(os.getpid()) + '-' + str(len(threads) + 1)
        threads[tid] = p
        log('thread', thread=tid, params=p)
        result = {'thread': {'id': tid}}
    elif method == 'turn/start':
        tid = p['threadId']
        cfg = threads[tid]
        brief = p['input'][0]['text']
        log('turn', thread=tid, params=p)
        send({'id': msg['id'], 'result': {'turn': {'id': tid + '-turn'}}})
        names = [t['name'] for t in cfg.get('dynamicTools', [])]
        if 'run_research' in names:
            call(tid, 'run_research', {'caller_role': 'core'} if mode == 'spoof' else {})
        elif 'run_plan' in names:
            call(tid, 'run_plan', {})
        elif 'dispatch_batch' in names:
            call(tid, 'dispatch_batch', {'unit_ids': ['u1', 'u2']})
        elif 'ROLE Research' in brief:
            log('research', thread=tid, effort=p.get('effort'), brief=brief)
            finish(tid, json.dumps({'summary': 'independent research', 'findings': ['freeze Python checks before writes']}))
        elif 'ROLE Planner' in brief:
            check = "from pathlib import Path; import sys,os; p=Path(sys.argv[1]); assert Path(os.environ['CARGO_TARGET_DIR']).name=='planned-target' and Path(os.environ['CARGO_TARGET_DIR']).parent.samefile(Path.cwd()), 'frozen target overwritten'; assert (p/'one.txt').read_text()=='one'; assert (p/'two.txt').read_text()=='two'; Path('scratch-proof').write_text('ok'); print('real frozen check passed')"
            if mode == 'failed-check':
                check = 'raise SystemExit(7)'
            plan = {'assembly_scope': None, 'units': [{'kind':'engineer','id': 'u1', 'brief': 'write one', 'scope': ['one.txt'], 'depends_on': []},
                              {'kind':'engineer','id': 'u2', 'brief': 'write two', 'scope': ['two.txt'], 'depends_on': []}],
                    'checks': [{'id': 'frozen-python', 'program': os.environ.get('AXIOM_TEST_PYTHON', sys.executable),
                                'args': ['-I', '-c', check, '{repo}'], 'env': [{'name':'CARGO_TARGET_DIR','value':'{scratch}/planned-target'}], 'timeout_s': 10}]}
            if mode == 'content':
                plan['units'][1].update(kind='content', scope=[])
                plan['assembly_scope'] = ['one.txt', 'two.txt']
            if mode == 'repairable-review':
                plan['assembly_scope'] = ['one.txt', 'two.txt', 'review-fix']
            finish(tid, json.dumps(plan))
        elif 'ROLE Engineer' in brief:
            log('engineer-start', thread=tid)
            env = cfg['config']['shell_environment_policy']['set']
            work = Path(env['AXIOM_SCRATCH'])
            assert work.is_absolute() and work.is_dir()
            assert Path(cfg['cwd']).resolve() not in work.resolve().parents
            assert Path(env['CARGO_TARGET_DIR']).parent.samefile(work)
            assert len(cfg['config']['sandbox_workspace_write']['writable_roots']) == 1
            (work/'development-output').write_text('scratch-only')
            if mode == 'failed-peer':
                if 'write one' in brief:
                    finish(tid, 'injected engineering failure', failed=True)
                # The sibling intentionally never completes without cancellation.
                continue
            if mode == 'unknown-write':
                Path(cfg['cwd'], 'one.txt').write_text('unknown')
                sys.exit(9)
            def engineer(t=tid, b=brief, cwd=cfg['cwd']):
                if mode != 'content':
                    barrier.wait()
                time.sleep(0.15)
                name, text = ('one.txt', 'one') if 'write one' in b else ('two.txt', 'two')
                Path(cwd, name).write_text(text)
                log('engineer-finish', thread=t)
                finish(t)
            threading.Thread(target=engineer, daemon=True).start()
        elif 'ROLE Content' in brief:
            log('content', thread=tid)
            finish(tid, 'two')
        elif 'ROLE Assembler' in brief:
            log('assembly', thread=tid)
            if mode == 'repairable-review' and 'Repair the independently rejected result' in brief:
                Path(cfg['cwd'], 'review-fix').write_text('fixed')
            if mode == 'content':
                Path(cfg['cwd'], 'two.txt').write_text('two')
            finish(tid)
        elif 'verify_run' in names:
            if mode == 'no-check' and 'baseline' not in brief.lower():
                finish(tid, json.dumps({'pass': True, 'summary': 'unsubstantiated claim'}))
            else:
                passed = mode != 'rejected-review' and (mode != 'repairable-review' or Path(cfg['cwd'], 'review-fix').exists())
                call(tid, 'verify_run', {'check_id': 'arbitrary' if mode == 'unknown-check' else 'frozen-python'}, json.dumps({'pass': passed, 'summary': 'checked independently'}))
        elif 'record_memory' in names:
            schema = cfg['dynamicTools'][0]['inputSchema']
            evidence = schema['properties']['evidence_id']['enum'][0]
            call(tid, 'record_memory', {'evidence_id': evidence, 'content': 'Verified two scoped units and mandatory assembly with frozen-python.'})
        else:
            finish(tid)
        continue
    elif method == 'command/exec':
        log('command', process_cwd=os.getcwd(), params=p)
        proc = subprocess.run(p['command'], cwd=p['cwd'], env=dict(os.environ, **p.get('env', {})), capture_output=True, text=True, timeout=p['timeoutMs'] / 1000)
        result = {'exitCode': proc.returncode, 'stdout': proc.stdout, 'stderr': proc.stderr, 'timedOut': False}
    elif method == 'thread/read':
        result = {'thread': {'id': p['threadId'], 'turns': []}}
    elif method == 'turn/interrupt':
        if mode == 'failed-peer':
            log('interrupted', thread=p['threadId'])
            finish(p['threadId'], 'cancelled after peer failure', failed=True)
        result = {}
    else:
        continue
    send({'id': msg['id'], 'result': result})
