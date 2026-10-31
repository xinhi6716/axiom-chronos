"""Small real-model smoke. Copies the binary before launch; only edits a disposable repo.
No install, push, native agents, heldout tasks or engineering rounds.
Use --fake for protocol-only harness validation, never claim it is real-model evidence.
"""
import argparse, json, os, queue, shutil, subprocess, sys, threading, time
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--exe', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--fake', action='store_true')
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    binary = out / ('axiom-smoke.exe' if os.name == 'nt' else 'axiom-smoke')
    shutil.copy2(args.exe.resolve(strict=True), binary)
    repo = out / 'project'
    home = out / 'home'
    repo.mkdir()
    verify = repo / 'verify_smoke.py'
    verify.write_text("from pathlib import Path\nassert Path('result.txt').read_text() == 'CHRONOS_SMOKE_OK'\n", encoding='utf-8')
    for cmd in [['git', 'init', '-q'], ['git', 'add', '.'], ['git', '-c', 'user.name=Smoke', '-c', 'user.email=smoke@example.invalid', 'commit', '-qm', 'fixture']]:
        subprocess.run(cmd, cwd=repo, check=True, capture_output=True)
    env = dict(os.environ, AXIOM_HOME=str(home), AXIOM_DATA_HOME=str(home))
    verification = '"' + sys.executable + '" -I "' + str(verify) + '"'
    if args.fake:
        env['AXIOM_CODEX_CMD'] = json.dumps([sys.executable, '-I', str(Path(__file__).resolve().parents[1] / 'bridge' / 'fake_dispatcher.py'), '--scenario', 'attached-smoke', '--home', str(home), '--project', str(repo), '--verify', verification, '--signal', str(out / 'signal')])
    errors = (out / 'bridge-stderr.txt').open('w', encoding='utf-8')
    proc = subprocess.Popen([str(binary), 'bridge'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors, encoding='utf-8', env=env)
    messages = queue.Queue()
    def reader():
        for line in proc.stdout:
            messages.put(json.loads(line))
    threading.Thread(target=reader, daemon=True).start()
    responses = []
    def call(id, arguments):
        proc.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': id, 'method': 'tools/call', 'params': {'name': 'session', 'arguments': arguments}}) + '\n')
        proc.stdin.flush()
        deadline = time.monotonic() + 240
        while True:
            result = messages.get(timeout=max(0.01, deadline - time.monotonic()))
            responses.append(result)
            if result.get('id') == id:
                assert not result.get('error'), result
                value = result['result']['structuredContent']
                assert value.get('status') == 'completed', value
                return value
    started = time.monotonic()
    try:
        begin = call(1, {'action': 'begin', 'project': str(repo), 'text': '[no-architect] Smoke only: dispatch Sol to write result.txt and run verify_smoke.py.', 'max_turns': 20})
        assert any(t['name'] == 'Agent' for t in begin['tools'])
        assert not any(t['name'] == 'chronos_codex_run' for t in begin['tools'])
        call(2, {'action': 'call', 'tool': 'goal', 'arguments': {'mode': 'change'}})
        prompt = f'''Smoke only; inherited root mode is change. Your native sandbox remains read-only, but use the supplied Host dynamic tools. Use project demo, cwd {repo}. Call chronos_orch status (Host has initialized demo); write a GOAL/SCOPE/VERIFY brief to the store for unit smoke, scope result.txt only, verify command {verification}. Register it with chronos_orch unit add smoke --track build --brief <absolute brief path>. Then call chronos_codex_run with project demo, unit smoke, role sol, cwd {repo}. Worker GOAL: write result.txt containing exactly CHRONOS_SMOKE_OK, no other edits. Return a bounded transport summary and artifact paths; no review, memory, commit, push, installation or further rounds. Report actual tool errors, never infer cannot-dispatch from native read-only mode.'''
        dispatch = call(3, {'action': 'call', 'tool': 'Agent', 'arguments': {'subagent_type': 'chronos:dispatcher', 'project': 'demo', 'cwd': str(repo), 'prompt': prompt}})
        assert len(json.dumps(dispatch).encode()) <= 8192
        assert 'final_message' not in dispatch
        assert (repo / 'result.txt').read_text() == 'CHRONOS_SMOKE_OK'
        subprocess.run([sys.executable, '-I', str(verify)], cwd=repo, check=True, capture_output=True)
        events = [json.loads(line) for p in (home / 'logs').glob('*.jsonl') for line in p.read_text(encoding='utf-8').splitlines()]
        assert not any(e.get('hostEvent') == 'threadStarted' and e.get('role') == 'core' for e in events)
        assert any(e.get('hostEvent') == 'threadStarted' and e.get('role') == 'dispatcher' and 'terra' in e.get('model', '') for e in events)
        assert any(e.get('hostEvent') == 'toolStarted' and e.get('role') == 'dispatcher' and e.get('tool') == 'chronos_codex_run' and e['arguments'].get('role') == 'sol' for e in events)
        worker = []
        for e in events:
            if e.get('hostEvent') == 'toolFinished' and e.get('tool') == 'chronos_codex_run':
                value = json.loads(e['result']['contentItems'][0]['text'])
                worker.append(value)
        assert any(w.get('state') == 'built' and w.get('verify_exit') == 0 and 'sol' in w.get('model', '') and w.get('effort') == 'high' for w in worker), worker
        summary = {'status': 'passed', 'evidence': 'fake-protocol-only' if args.fake else 'real-model-dispatch-smoke', 'wall_s': round(time.monotonic() - started, 3), 'dispatcher': dispatch, 'worker': worker, 'installed_acceptance': False, 'full_workflow_acceptance': False}
        (out / 'summary.json').write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding='utf-8')
        print(json.dumps({'status': 'passed', 'evidence': summary['evidence'], 'summary': str(out / 'summary.json')}))
    finally:
        (out / 'responses.json').write_text(json.dumps(responses, ensure_ascii=False, indent=2), encoding='utf-8')
        proc.stdin.close()
        try:
            proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=5)
        errors.close()


if __name__ == '__main__':
    main()
