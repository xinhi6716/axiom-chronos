"""RPC regression fixture, not live acceptance or a production execution path."""
import json
import hashlib
import os
import subprocess
import sys
import threading
import time
import traceback
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
PARENT_CHECK = "from pathlib import Path; import sys; p=Path(sys.argv[1]); assert (p/'first'/'one.txt').read_text()=='one'; assert (p/'second'/'two.txt').read_text()=='two'"
PRIVATE_CHECK = "from pathlib import Path; import sys; private_case='PRIVATE_EVALUATION_CANARY'; p=Path(sys.argv[1]); assert (p/'one.txt').read_text()+'|'+(p/'two.txt').read_text()=='one|two'"
ENV_CHECK = "from pathlib import Path; import sys,os; p=Path(sys.argv[1]); name='one.txt' if os.environ['AXIOM_CASE']=='public' else 'two.txt'; assert (p/name).read_text() in ('one','two')"
CASE_CHECK = "from pathlib import Path; import sys,os; p=Path(sys.argv[1]); name='one.txt' if 'AXIOM_CASE' in os.environ else 'two.txt'; assert (p/name).read_text() in ('one','two')"
UNICODE_CASE_CHECK = "from pathlib import Path; import sys,os; p=Path(sys.argv[1]); name='one.txt' if 'AXIOM_ÉTAT' in os.environ else 'two.txt'; assert (p/name).read_text() in ('one','two')"
FIRST_RECOVERY = 'first correction: use independent-h for the private ID'
SECOND_RECOVERY = 'second correction: prepare an inline acceptance check'
RUNNER_SOURCE = r'''use std::{env, fs, path::Path};
fn main() {
    let mut args = env::args().skip(1);
    let root = args.next().unwrap();
    while let Some(name) = args.next() {
        let expected = args.next().unwrap();
        assert_eq!(fs::read_to_string(Path::new(&root).join(name)).unwrap(), expected);
    }
}'''


def send(value):
    with lock:
        print(json.dumps(value), flush=True)


def log(kind, **values):
    with lock:
        with log_path.open('a', encoding='utf-8') as out:
            out.write(json.dumps(dict(kind=kind, time=time.monotonic(), **values)) + '\n')


def fatal(exc_type, exc, tb):
    # A fixture crash would otherwise look like a lost connection. Keep the
    # traceback in the replay log so the failing test can name the real cause.
    try:
        log('fixture-error', traceback=''.join(traceback.format_exception(exc_type, exc, tb)))
    finally:
        sys.__excepthook__(exc_type, exc, tb)


def thread_fatal(args):
    try:
        log('fixture-thread-error', thread=args.thread.name if args.thread else None,
            traceback=''.join(traceback.format_exception(args.exc_type, args.exc_value, args.exc_traceback)))
    finally:
        threading.__excepthook__(args)


sys.excepthook = fatal
threading.excepthook = thread_fatal


def finish(tid, text='done', failed=False, cancelled=False):
    turn = tid + '-turn'
    send({'method': 'item/completed', 'params': {'threadId': tid, 'turnId': turn,
          'item': {'type': 'agentMessage', 'id': 'message', 'text': text}}})
    send({'method': 'turn/completed', 'params': {'threadId': tid,
          'turn': {'id': turn, 'status': 'failed' if failed else 'cancelled' if cancelled else 'completed'}}})


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
    if mode in ['evaluation-reordered-env', 'evaluation-distinct-env']:
        return [{'id': 'independent-output', 'program': os.environ.get('AXIOM_TEST_PYTHON', sys.executable),
                 'args': ['-I', '-c', ENV_CHECK, '{repo}'],
                 'env': [{'name': 'AXIOM_CASE', 'value': 'public' if mode == 'evaluation-reordered-env' else 'private'},
                         {'name': 'CARGO_TARGET_DIR', 'value': '{scratch}/planned-target'}], 'timeout_s': 10}]
    if mode in ['evaluation-case-variant', 'evaluation-unicode-case-variant']:
        name = 'axiom_case' if mode == 'evaluation-case-variant' else 'axiom_état'
        check = CASE_CHECK if mode == 'evaluation-case-variant' else UNICODE_CASE_CHECK
        return [{'id': 'independent-output', 'program': os.environ.get('AXIOM_TEST_PYTHON', sys.executable),
                 'args': ['-I', '-c', check, '{repo}'],
                 'env': [{'name': name, 'value': 'same'},
                         {'name': 'CARGO_TARGET_DIR', 'value': '{scratch}/planned-target'}], 'timeout_s': 10}]
    if mode == 'private-duplicate-env':
        return [{'id': 'independent-output', 'program': sys.executable,
                 'args': ['-I', '-c', PRIVATE_CHECK, '{repo}'],
                 'env': [{'name': 'AXIOM_CASE', 'value': 'a'}, {'name': 'AXIOM_CASE', 'value': 'b'}],
                 'timeout_s': 10}]
    return [{'id': 'independent-h' if mode in ['planning-retry', 'retry-second-epoch'] else 'independent-output', 'program': sys.executable,
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
    fixed = Path(threads[tid]['cwd'], 'review-fix')
    passed = mode != 'rejected-review' and (mode not in ['repairable-review', 'evaluation-repairable-review', 'repairable-review-twice'] or fixed.exists())
    if mode == 'repairable-review-twice':
        passed = fixed.exists() and fixed.read_text() == 'twice'
    return json.dumps({'pass': passed, 'summary': 'checked independently', 'defect_owner': 'assembly' if not passed else 'none'})


for line in sys.stdin:
    msg = json.loads(line)
    method, p = msg.get('method'), msg.get('params', {})
    if not method:
        if msg.get('id') in pending:
            tid, final = pending.pop(msg['id'])
            log('reply', thread=tid, response=msg)
            ok = reply_ok(msg)
            if final == '__prepared_runner__':
                if not ok:
                    finish(tid, 'runner callback failed', failed=True)
                    continue
                prepared = json.loads(msg['result']['contentItems'][0]['text'])
                log('runner-result', status=prepared.get('status'))
                if prepared.get('status') == 'ready':
                    checks = [{'id': 'independent-output', 'program': prepared['program'],
                               'args': ['{repo}', 'one.txt', 'one', 'two.txt', 'two'], 'env': [], 'timeout_s': 30}]
                    finish(tid, json.dumps({'status': 'ready', 'summary': 'prepared',
                                            'exposure': 'synthetic private acceptance', 'checks': checks}))
                else:
                    # Deliberate adversarial fallback: Rust must still block.
                    finish(tid, preparation())
                continue
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
            if mode == 'verifier-history-null' and 'verify_run' in [t['name'] for t in threads[tid].get('dynamicTools', [])]:
                log_path.with_suffix('.model-history').write_text(json.dumps({'thread': tid, 'final': final}), encoding='utf-8')
                log('model-history-disconnect', thread=tid)
                sys.exit(19)
            finish(tid, final, not ok)
        continue
    if method == 'initialize':
        result = {}
    elif method == 'model/list':
        roles = ['terra', 'sol', 'astra', 'luna']
        if mode == 'verifier-history-null' and log_path.with_suffix('.model-history').exists():
            roles.remove('astra')  # Catalog changes after the verifier has run.
        log('model-list', roles=roles)
        result = {'data': [{'id': 'gpt-6.1-' + role,
                  'supportedReasoningEfforts': [{'reasoningEffort': e} for e in ['medium', 'high', 'xhigh', 'max']]}
                  for role in roles]}
    elif method == 'plugin/installed':
        result = {'marketplaces': []}
    elif method == 'config/read':
        result = {'config': {}}
    elif method == 'thread/start':
        tid = 't-' + str(os.getpid()) + '-' + str(len(threads) + 1)
        threads[tid] = p
        log('thread', thread=tid, params=p)
        result = {'thread': {'id': tid}, 'model': p['model']}
        if mode.startswith('engineer-config-') and p.get('sandbox') == 'workspace-write' and not log_path.with_suffix('.fixed-model').exists():
            if mode == 'engineer-config-missing':
                del result['model']
            else:
                result['model'] = 'different-configured-model'
    elif method == 'turn/start':
        tid = p['threadId']
        cfg = threads[tid]
        brief = p['input'][0]['text']
        log('turn', thread=tid, params=p)
        names = [t['name'] for t in cfg.get('dynamicTools', [])]
        # Fault injection happens before acknowledging turn/start. Rust has
        # already recorded BeforeTurn and cannot record TurnStarted until this
        # response, so no live checkpoint mutation races this test fixture.
        if 'run_plan' in names and mode in ['retry-missing', 'retry-tampered', 'retry-wrong-epoch'] and log_path.with_suffix('.failed-plan').exists():
            run_dirs = list((log_path.parent / 'home' / 'orchestrate' / 'demo' / 'runs').iterdir())
            assert len(run_dirs) == 1
            checkpoint_path = run_dirs[0] / 'checkpoint.json'
            checkpoint = json.loads(checkpoint_path.read_text(encoding='utf-8'))
            receipt = checkpoint['receipts']['planning-retry-1']
            receipt_path = Path(receipt['path'])
            if mode == 'retry-missing':
                del checkpoint['receipts']['planning-retry-1']
            elif mode == 'retry-tampered':
                receipt_path.write_text('tampered', encoding='utf-8')
            else:
                altered = json.loads(receipt_path.read_text(encoding='utf-8'))
                altered['epoch'] = 2
                data = json.dumps(altered).encode('utf-8')
                receipt_path.write_bytes(data)
                new_digest = hashlib.sha256(data).hexdigest()
                for key, value in checkpoint['receipts'].items():
                    if key == 'planning-retry-1' or key.startswith('planning-retired-'):
                        value['digest'] = new_digest
            # Atomic replacement prevents readers from seeing a partial file.
            new_checkpoint = checkpoint_path.with_name('checkpoint-fixture.tmp')
            new_checkpoint.write_text(json.dumps(checkpoint), encoding='utf-8')
            os.replace(new_checkpoint, checkpoint_path)
        send({'id': msg['id'], 'result': {'turn': {'id': tid + '-turn'}}})
        if 'run_research' in names:
            call(tid, 'run_research', {'caller_role': 'core'} if mode == 'spoof' else {})
        elif 'run_plan' in names:
            call(tid, 'run_plan', {})
        elif 'dispatch_batch' in names:
            if mode == 'engineer-config-retry-parent-failure':
                marker = log_path.with_suffix('.batch-coordinator-a')
                if marker.exists():
                    log('coordinator-failed-before-batch', thread=tid)
                    finish(tid, 'coordinator failed before invoking dispatch_batch', failed=True)
                    continue
                marker.write_text(tid, encoding='utf-8')
            ids = ['u1'] if mode in ['unknown-then-failed', 'unknown-partial', 'unready-peer-write', 'unknown-completed-inside', 'unknown-completed-outside', 'unknown-history-cancelled', 'unknown-history-failed', 'unknown-no-turn', 'writer-cancelled'] else ['u1', 'u2']
            call(tid, 'dispatch_batch', {'unit_ids': ids})
        elif 'ROLE Research' in brief:
            log('research', thread=tid, effort=p.get('effort'), brief=brief)
            finish(tid, json.dumps({'summary': 'independent research', 'findings': ['freeze Python checks before writes']}))
        elif 'ROLE Planner' in brief:
            check = PUBLIC_CHECK
            if mode == 'failed-check':
                check = 'raise SystemExit(7)'
            if mode == 'directory-peers':
                check = PARENT_CHECK
            plan = {'independent_evaluation': 'Check the two outputs independently before release' if mode in ['evaluation', 'evaluation-repairable-review', 'evaluation-reordered-env', 'evaluation-distinct-env', 'evaluation-case-variant', 'evaluation-unicode-case-variant', 'evaluation-lowercase-default', 'public-duplicate-case-env', 'public-duplicate-env', 'private-duplicate-env', 'planning-retry', 'retry-second-epoch', 'retry-missing', 'retry-tampered', 'retry-wrong-epoch', 'public-private-id', 'evaluation-recovered', 'evaluation-unprepared', 'evaluation-fake-ready', 'baseline-infrastructure', 'runner-ready', 'runner-fail-fallback', 'runner-forged', 'runner-unknown'] else None, 'assembly_scope': None, 'units': [{'kind':'engineer','id': 'u1', 'brief': 'write one', 'scope': ['one.txt'], 'depends_on': []},
                              {'kind':'engineer','id': 'u2', 'brief': 'write two', 'scope': ['two.txt'], 'depends_on': []}],
                    'checks': [{'id': 'frozen-python', 'program': os.environ.get('AXIOM_TEST_PYTHON', sys.executable),
                                'args': ['-I', '-c', check, '{repo}'], 'env': [{'name':'CARGO_TARGET_DIR','value':'{scratch}/planned-target'}], 'timeout_s': 10}]}
            if mode == 'memory-no-query':
                # This is a valid nonempty unit brief but has no search terms.
                plan['units'][1]['brief'] = '+'
            if mode in ['unknown-then-failed', 'unknown-partial']:
                plan['units'] = plan['units'][:1]
            if mode == 'unready-peer-write':
                plan['units'][1]['depends_on'] = ['u1']
            if mode == 'directory-peers':
                plan['units'][0]['scope'] = ['first/one.txt']
                plan['units'][1]['scope'] = ['second/two.txt']
            if mode == 'content':
                plan['units'][1].update(kind='content', scope=[])
                plan['assembly_scope'] = ['one.txt', 'two.txt']
            if mode in ['repairable-review', 'evaluation-repairable-review', 'repairable-review-twice']:
                plan['assembly_scope'] = ['one.txt', 'two.txt', 'review-fix']
            if mode in ['unknown-completed-inside', 'unknown-completed-outside', 'unknown-history-cancelled', 'unknown-history-failed', 'unknown-no-turn', 'writer-cancelled']:
                plan['units'][1]['depends_on'] = ['u1']
            if mode == 'public-private-id':
                plan['checks'][0]['id'] = 'independent-public'
            if mode == 'runner-public-forged':
                plan['checks'][0]['program'] = '@axiom:rust:v1:' + '0'*64 + ':' + '1'*64
            if mode in ['evaluation-reordered-env', 'evaluation-distinct-env']:
                plan['checks'][0]['args'][2] = ENV_CHECK
                plan['checks'][0]['env'].append({'name': 'AXIOM_CASE', 'value': 'public'})
            if mode in ['evaluation-case-variant', 'evaluation-unicode-case-variant']:
                name = 'AXIOM_CASE' if mode == 'evaluation-case-variant' else 'AXIOM_ÉTAT'
                plan['checks'][0]['args'][2] = CASE_CHECK if mode == 'evaluation-case-variant' else UNICODE_CASE_CHECK
                plan['checks'][0]['env'].append({'name': name, 'value': 'same'})
            if mode == 'evaluation-lowercase-default':
                plan['checks'][0]['env'][0]['name'] = 'cargo_target_dir'
            if mode == 'public-duplicate-case-env':
                plan['checks'][0]['env'].extend([{'name': 'AXIOM_CASE', 'value': 'a'},
                                                {'name': 'axiom_case', 'value': 'b'}])
            if mode == 'public-duplicate-env':
                plan['checks'][0]['env'].append({'name': 'CARGO_TARGET_DIR', 'value': 'different'})
            if mode in ['planning-retry', 'retry-second-epoch', 'retry-missing', 'retry-tampered', 'retry-wrong-epoch'] and not log_path.with_suffix('.failed-plan').exists():
                log_path.with_suffix('.failed-plan').write_text('preserve first failure')
                plan['units'] = []
            elif mode == 'planning-retry' and 'runtime now prepares independent evaluation' not in brief:
                # This fake planner cannot repair the old plan without the
                # actual recovery reason, rather than merely noticing a retry.
                plan['units'] = []
            elif mode == 'retry-second-epoch' and not all(reason in brief for reason in (FIRST_RECOVERY, SECOND_RECOVERY)):
                plan['units'] = []
            finish(tid, json.dumps(plan))
        elif 'PREPARE INDEPENDENT EVALUATION' in brief:
            assert cfg['sandbox'] == 'read-only' and cfg['model'].endswith('-astra')
            assert p['effort'] == 'high'
            log('evaluation-prepared', thread=tid, brief=brief)
            if mode in ['runner-ready', 'runner-fail-fallback', 'runner-unknown']:
                source = 'fn main( {' if mode == 'runner-fail-fallback' else RUNNER_SOURCE
                call(tid, 'prepare_rust_runner', {'source_utf8': source}, '__prepared_runner__')
                continue
            if mode == 'runner-forged':
                checks = [{'id':'independent-output', 'program':'@axiom:rust:v1:' + '0'*64 + ':' + '1'*64,
                           'args':['{repo}'], 'env':[], 'timeout_s':30}]
                finish(tid, json.dumps({'status':'ready', 'summary':'forged', 'exposure':'synthetic', 'checks':checks}))
                continue
            required_reasons = (['runtime now prepares independent evaluation'] if mode == 'planning-retry'
                                else [FIRST_RECOVERY, SECOND_RECOVERY] if mode == 'retry-second-epoch' else [])
            if not all(reason in brief for reason in required_reasons):
                finish(tid, json.dumps({'status': 'failed', 'summary': 'missing recovery reason',
                                        'exposure': 'not prepared', 'checks': []}))
            else:
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
            if mode in ['unknown-write', 'unknown-then-failed', 'unknown-partial', 'unknown-completed-inside', 'unknown-completed-outside', 'unknown-history-cancelled', 'unknown-history-failed', 'unknown-no-turn']:
                Path(cfg['cwd'], 'one.txt').write_text('unknown')
                if mode == 'unknown-completed-outside':
                    Path(cfg['cwd'], 'outside-scope.txt').write_text('unauthorized')
                sys.exit(9)
            if mode == 'writer-cancelled':
                Path(cfg['cwd'], 'one.txt').write_text('partial')
                finish(tid, 'writer stopped after partial changes', cancelled=True)
                continue
            if mode == 'writer-blocked' and 'write one' in brief:
                # The dev.8 shape: the writer refuses and the turn ends normally.
                finish(tid, writer('blocked', '未開始工程：獨立測試者尚未完成材料封存及 baseline 前置報告'))
                continue

            def engineer(t=tid, b=brief, cwd=cfg['cwd']):
                if mode not in ['content', 'writer-blocked', 'unready-peer-write']:
                    try:
                        barrier.wait()
                    except threading.BrokenBarrierError:
                        # A loaded runner must not turn a rendezvous timeout
                        # into a lost execution; record it and keep going.
                        log('barrier-timeout', thread=t)
                time.sleep(0.15)
                name, text = ('one.txt', 'one') if 'write one' in b else ('two.txt', 'two')
                if mode == 'unready-peer-write':
                    name, text = 'two.txt', 'outside-scope'
                elif mode == 'directory-peers':
                    name = ('first/' if 'write one' in b else 'second/') + name
                target = Path(cwd, name)
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text(text)
                log('engineer-finish', thread=t)
                finish(t, writer('completed', f'wrote {name}', None))
            threading.Thread(target=engineer, daemon=True).start()
        elif 'ROLE Content' in brief:
            log('content', thread=tid)
            finish(tid, writer('completed', 'requested artifact text', 'two'))
        elif 'ROLE Assembler' in brief:
            log('assembly', thread=tid)
            if mode in ['repairable-review', 'evaluation-repairable-review', 'repairable-review-twice'] and 'Repair the independently rejected result' in brief:
                fix = Path(cfg['cwd'], 'review-fix')
                fix.write_text('twice' if mode == 'repairable-review-twice' and fix.exists() else 'fixed')
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
            if mode == 'memory-forged':
                evidence = 'not-a-review-digest'
            if mode == 'memory-stale':
                # Simulate an external source change after review, before the
                # read-only memory role's callback is processed.
                Path(cfg['cwd'], 'one.txt').write_text('changed after review')
            content = ('write one VERIFIED_MEMORY_CANARY' if mode == 'memory-reuse'
                       else 'Verified two scoped units and mandatory assembly with frozen-python.')
            call(tid, 'record_memory', {'evidence_id': evidence, 'content': content})
        else:
            finish(tid)
        continue
    elif method == 'command/exec':
        log('command', process_cwd=os.getcwd(), params=p)
        if mode == 'runner-unknown' and p['command'][1:] == ['-vV']:
            os._exit(19)
        proc = subprocess.run(p['command'], cwd=p['cwd'], env=dict(os.environ, **p.get('env', {})), capture_output=True, text=True, encoding='utf-8', errors='replace', timeout=p['timeoutMs'] / 1000)
        result = {'exitCode': proc.returncode, 'stdout': proc.stdout, 'stderr': proc.stderr, 'timedOut': False}
    elif method == 'thread/read':
        model_history = log_path.with_suffix('.model-history')
        if mode == 'verifier-history-null' and model_history.exists():
            saved = json.loads(model_history.read_text(encoding='utf-8'))
            if p['threadId'] == saved['thread']:
                log('model-history-read', thread=p['threadId'])
                send({'id': msg['id'], 'result': {'thread': {'id': p['threadId'], 'model': None,
                      'turns': [{'id': p['threadId'] + '-turn', 'status': 'completed',
                                 'items': [{'type': 'agentMessage', 'id': 'history-verifier', 'text': saved['final']}]}]}}})
                continue
        history_marker = log_path.with_name(log_path.name + '.' + p['threadId'] + '.history-failed')
        terminal = mode in ['unknown-then-failed', 'unknown-completed-inside', 'unknown-completed-outside', 'unknown-history-cancelled', 'unknown-history-failed', 'unknown-no-turn'] or (mode == 'unknown-partial' and not history_marker.exists())
        if terminal:
            history_marker.write_text('one terminal observation', encoding='utf-8')
            log('history-failed', thread=p['threadId'])
            status = ('completed' if mode in ['unknown-completed-inside', 'unknown-completed-outside', 'unknown-no-turn']
                      else 'cancelled' if mode == 'unknown-history-cancelled' else 'failed')
            turns = [{'id': p['threadId'] + '-turn', 'status': status,
                      'items': [{'type': 'agentMessage', 'id': 'history-message', 'text': writer()}],
                      'error': {'message': 'terminal observed from history'} if status == 'failed' else None}]
        else:
            turns = []
        result = {'thread': {'id': p['threadId'], 'turns': turns}}
    elif method == 'turn/interrupt':
        if mode == 'failed-peer':
            log('interrupted', thread=p['threadId'])
            finish(p['threadId'], 'cancelled after peer failure', failed=True)
        result = {}
    else:
        continue
    send({'id': msg['id'], 'result': result})
