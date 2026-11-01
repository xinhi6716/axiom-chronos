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
verify_queue = {}
baseline = {}
call_counts = {}
barrier = threading.Barrier(2, timeout=10)

PUBLIC_CHECK = "from pathlib import Path; import sys,os; p=Path(sys.argv[1]); assert Path(os.environ['CARGO_TARGET_DIR']).name=='planned-target' and Path(os.environ['CARGO_TARGET_DIR']).parent.samefile(Path.cwd()), 'frozen target overwritten'; assert (p/'one.txt').read_text()=='one'; assert (p/'two.txt').read_text()=='two'; Path('scratch-proof').write_text('ok'); print('real frozen check passed')"
PRIVATE_CHECK = "from pathlib import Path; import sys; private_case='PRIVATE_EVALUATION_CANARY'; p=Path(sys.argv[1]); assert (p/'one.txt').read_text()+'|'+(p/'two.txt').read_text()=='one|two'"


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


def call(tid, name, args, final='done', schema=None):
    call_counts[tid] = call_counts.get(tid, 0) + 1
    cid = 'tool-' + tid + '-' + str(call_counts[tid])
    pending[cid] = (tid, final if schema is None else json.dumps(schema))
    send({'id': cid, 'method': 'item/tool/call', 'params': {
        'threadId': tid, 'turnId': tid + '-turn', 'callId': cid,
        'tool': name, 'arguments': args}})


def writer(status='completed', summary='done', artifact=None):
    """Typed business result of a writing leaf (see the role contract)."""
    return json.dumps({'status': status, 'summary': summary, 'artifact': artifact})


def reply_ok(msg):
    return 'error' not in msg and msg.get('result', {}).get('success')


def check_exit_code(msg):
    items = msg.get('result', {}).get('contentItems') or []
    if not items:
        return None
    return json.loads(items[0]['text']).get('exit_code')


def baseline_classification(exit_code):
    if mode == 'baseline-infrastructure':
        return 'infrastructure_failure'
    return 'passed' if exit_code == 0 else 'product_missing'


def baseline_result(tid):
    state = baseline[tid]
    observations = [{'check_id': cid, 'classification': baseline_classification(code)}
                    for cid, code in zip(state['ids'], state['codes'])]
    return json.dumps({'infrastructure_ready': mode != 'baseline-infrastructure',
                       'summary': 'baseline executed every frozen check',
                       'observations': observations})


def independent_checks():
    """Private acceptance checks: fresh cases, product-inspecting commands."""
    if mode == 'evaluation-fake-ready':
        # A public regression command relabelled as held-out acceptance.
        return [{'id': 'independent-copy', 'program': os.environ.get('AXIOM_TEST_PYTHON', sys.executable),
                 'args': ['-I', '-c', PUBLIC_CHECK, '{repo}'],
                 'env': [{'name': 'CARGO_TARGET_DIR', 'value': '{scratch}/planned-target'}], 'timeout_s': 10}]
    return [{'id': 'independent-output', 'program': sys.executable,
             'args': ['-I', '-c', PRIVATE_CHECK, '{repo}'], 'env': [], 'timeout_s': 10}]


def preparation():
    """Typed preparation result: ready only with genuine held-out checks."""
    marker = log_path.with_suffix('.prepared')
    if mode == 'evaluation-unprepared':
        # The dev.8 shape: the tester says it failed but still returns a check.
        return json.dumps({'status': 'failed',
                           'summary': 'PREPARATION_FAILED: private isolation and sealing were not available to this verifier',
                           'exposure': 'no case stays private; preparation did not complete',
                           'checks': [{'id': 'independent-preparation-gate', 'program': 'powershell.exe',
                                       'args': ['-NoProfile', '-NonInteractive', '-Command',
                                                "[Console]::Error.WriteLine('PREPARATION_FAILED'); exit 1"],
                                       'env': [], 'timeout_s': 10}]})
    if mode == 'evaluation-recovered' and not marker.exists():
        marker.write_text('first planning epoch failed to prepare evaluation')
        return json.dumps({'status': 'failed', 'summary': 'private runner unavailable on the first planning epoch',
                           'exposure': 'nothing prepared', 'checks': []})
    marker.write_text('prepared')
    return json.dumps({'status': 'ready', 'summary': 'new private evaluation prepared',
                       'exposure': 'private cases stay in this report; writer briefs never receive them',
                       'checks': independent_checks()})


def verifier_result(tid, brief):
    if mode == 'verifier-upstream':
        return json.dumps({'pass': False, 'summary': 'the frozen plan required a prerequisite no writer could supply',
                           'defect_owner': 'upstream'})
    if mode == 'verifier-no-owner':
        return json.dumps({'pass': False, 'summary': 'something is wrong', 'defect_owner': 'none'})
    passed = mode != 'rejected-review' and (mode != 'repairable-review' or Path(threads[tid]['cwd'], 'review-fix').exists())
    return json.dumps({'pass': passed, 'summary': 'checked independently', 'defect_owner': 'assembly' if not passed else 'none'})


for line in sys.stdin:
    msg = json.loads(line)
    method, p = msg.get('method'), msg.get('params', {})
    if not method:
        if msg.get('id') in pending:
            tid, final = pending.pop(msg['id'])
            log('reply', thread=tid, response=msg)
            ok = reply_ok(msg)
            if tid in baseline:
                baseline[tid]['codes'].append(check_exit_code(msg) if ok else None)
                if ok and len(baseline[tid]['codes']) < len(baseline[tid]['ids']):
                    call(tid, 'verify_run', {'check_id': baseline[tid]['ids'][len(baseline[tid]['codes'])]})
                    continue
                finish(tid, baseline_result(tid), not ok)
                continue
            if verify_queue.get(tid) and ok:
                call(tid, 'verify_run', {'check_id': verify_queue[tid].pop(0)}, final)
                continue
            finish(tid, final, not ok)
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
            check = PUBLIC_CHECK
            if mode == 'failed-check':
                check = 'raise SystemExit(7)'
            plan = {'independent_evaluation': 'Check the two outputs independently before release' if mode in ['evaluation', 'planning-retry', 'evaluation-recovered', 'evaluation-unprepared', 'evaluation-fake-ready', 'baseline-infrastructure'] else None, 'assembly_scope': None, 'units': [{'kind':'engineer','id': 'u1', 'brief': 'write one', 'scope': ['one.txt'], 'depends_on': []},
                              {'kind':'engineer','id': 'u2', 'brief': 'write two', 'scope': ['two.txt'], 'depends_on': []}],
                    'checks': [{'id': 'frozen-python', 'program': os.environ.get('AXIOM_TEST_PYTHON', sys.executable),
                                'args': ['-I', '-c', check, '{repo}'], 'env': [{'name':'CARGO_TARGET_DIR','value':'{scratch}/planned-target'}], 'timeout_s': 10}]}
            if mode == 'content':
                plan['units'][1].update(kind='content', scope=[])
                plan['assembly_scope'] = ['one.txt', 'two.txt']
            if mode == 'repairable-review':
                plan['assembly_scope'] = ['one.txt', 'two.txt', 'review-fix']
            if mode == 'planning-retry' and not log_path.with_suffix('.failed-plan').exists():
                log_path.with_suffix('.failed-plan').write_text('preserve first failure')
                plan['units'] = []
            finish(tid, json.dumps(plan))
        elif 'PREPARE INDEPENDENT EVALUATION' in brief:
            assert cfg['sandbox'] == 'read-only' and cfg['model'].endswith('-astra')
            assert p['effort'] == 'high'
            log('evaluation-prepared', thread=tid)
            finish(tid, preparation())
        elif 'ROLE Engineer' in brief:
            log('engineer-start', thread=tid)
            env = cfg['config']['shell_environment_policy']['set']
            work = Path(env['AXIOM_SCRATCH'])
            assert work.is_absolute() and work.is_dir()
            assert Path(cfg['cwd']).resolve() not in work.resolve().parents
            assert Path(env['CARGO_TARGET_DIR']).parent.samefile(work)
            assert len(cfg['config']['sandbox_workspace_write']['writable_roots']) == 1
            (work/'development-output').write_text('scratch-only')
            send({'method': 'item/completed', 'params': {'threadId': tid, 'turnId': tid + '-turn',
                  'item': {'type': 'fileChange', 'id': 'scratch-write', 'status': 'completed',
                           'changes': [{'path': str(work/'development-output'), 'kind': {'type': 'add'}, 'diff': ''}]}}})
            if mode == 'failed-peer':
                if 'write one' in brief:
                    finish(tid, 'injected engineering failure', failed=True)
                # The sibling intentionally never completes without cancellation.
                continue
            if mode == 'unknown-write':
                Path(cfg['cwd'], 'one.txt').write_text('unknown')
                sys.exit(9)
            if mode == 'writer-blocked' and 'write one' in brief:
                # The dev.8 shape: the writer refuses and the turn ends normally.
                finish(tid, writer('blocked', '未開始工程：獨立測試者尚未完成材料封存及 baseline 前置報告'))
                continue

            def engineer(t=tid, b=brief, cwd=cfg['cwd']):
                if mode not in ['content', 'writer-blocked']:
                    barrier.wait()
                time.sleep(0.15)
                name, text = ('one.txt', 'one') if 'write one' in b else ('two.txt', 'two')
                Path(cwd, name).write_text(text)
                log('engineer-finish', thread=t)
                finish(t, writer('completed', f'wrote {name}', None))
            threading.Thread(target=engineer, daemon=True).start()
        elif 'ROLE Content' in brief:
            log('content', thread=tid)
            finish(tid, writer('completed', 'requested artifact text', 'two'))
        elif 'ROLE Assembler' in brief:
            log('assembly', thread=tid)
            if mode == 'repairable-review' and 'Repair the independently rejected result' in brief:
                Path(cfg['cwd'], 'review-fix').write_text('fixed')
            if mode == 'content':
                Path(cfg['cwd'], 'two.txt').write_text('two')
            finish(tid, writer('completed', 'assembled the units', None))
        elif 'verify_run' in names:
            if 'Execute frozen baseline checks' in brief:
                ids = json.loads(brief.rsplit('VERIFY\n', 1)[1])
                baseline[tid] = {'ids': ids, 'codes': []}
                call(tid, 'verify_run', {'check_id': ids[0]})
            else:
                ids = json.loads(brief.rsplit('VERIFY\n', 1)[1])
                if mode == 'no-check':
                    # Claims a pass without invoking any frozen check.
                    finish(tid, verifier_result(tid, brief))
                    continue
                verify_queue[tid] = ids[1:]
                call(tid, 'verify_run', {'check_id': 'arbitrary' if mode == 'unknown-check' else ids[0]},
                     verifier_result(tid, brief))
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
