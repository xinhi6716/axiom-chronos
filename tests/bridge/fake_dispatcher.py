"""Protocol fixture: read-only attached dispatcher with actual Host dynamic callbacks."""
from pathlib import Path
source = (Path(__file__).parent.parent / 'fakes' / 'fake_codex_session.py').read_text(encoding='utf-8')
# Echo the requested sandbox in thread/start's authorization confirmation. The
# dispatcher is read-only, but its nested Sol worker is workspace-write.
anchor = "'workspaceWrite' if scenario=='unsafe-sandbox' else 'readOnly'"
assert source.count(anchor) == 1
source = source.replace(anchor, "'workspaceWrite' if params['sandbox']=='workspace-write' else 'readOnly'")
source = source.replace("record = option('--record', os.getenv('FAKE_CODEX_RECORD'))", "record = option('--record', os.getenv('FAKE_CODEX_RECORD'))\nif record: record += '.' + str(os.getpid()) + '.json'")
anchor = "    elif method in ('thread/start','thread/resume'):"
source = source.replace(anchor, "    elif method=='skills/list': reply(msg,{'data':[{'cwd':params['cwds'][0],'errors':[],'skills':[]}]})\n" + anchor)
anchor = "    role=threads[tid].get('model','')"
scenario = r'''    if scenario.startswith('pipeline'):
        spec=threads[tid]
        if scenario=='pipeline-dispatcher' and any(t['name']=='chronos_codex_run' for t in spec.get('dynamicTools',[])):
            result=tool(tid,turn,'codex_run',{'project':'demo','unit':'checkpointed','role':'terra','cwd':spec['cwd']})
            assert result['success'] or option('--fault','').startswith('host-callback-'), result
            if option('--fault','')=='host-callback-test-findings':
                data=json.loads(result['contentItems'][0]['text'])
                assert data['outcome']=='repair_required',data
                prepared=tool(tid,turn,'orch',{'project':'demo','args':['wait','prepare','--task','checkpointed','--repo',spec['cwd'],'--issue','actual test failure','--proposal','repair verified finding','--min-rounds','5']})
                assert prepared['success'],prepared
            if option('--fault','').startswith('host-callback-final'):
                full=tool(tid,turn,'orch',{'project':'demo','args':['fulltest','--repo',spec['cwd'],'--command','cargo test --locked --offline']})
                assert full['success'], full
                tested=tool(tid,turn,'Agent',{'subagent_type':'chronos:tester','project':'demo','cwd':spec['cwd'],'prompt':'Verify current assembled product and stored test evidence.'})
                assert tested['success'] == (option('--fault','')!='host-callback-final-reject'), tested
            complete(tid,turn,'Bounded host round completed')
            return
        if 'You are the final independent tester' in spec.get('developerInstructions',''):
            assert spec['sandbox']=='read-only'
            assert params.get('effort')=='high'
            issues=['Final acceptance evidence missing'] if option('--fault','')=='host-callback-final-reject' else []
            complete(tid,turn,json.dumps({'issues':issues,'severity':'normal','evidence':'Deterministic model fixture; real Cargo test/package evidence is checked by Host.'}))
            return
        role=spec['model']
        text=params['input'][0]['text']
        if 'astra' in role:
            instructions=spec.get('developerInstructions','')
            assert 'Do not run Cargo' in instructions and 'build, test' in instructions
            assert 'Do not run Cargo' in text and '\nVERIFY ' not in text
            if params.get('outputSchema'):
                assert 'Host verification receipt:' in text and 'Do not run Cargo' in text
        fault=option('--fault','')
        if fault.startswith('host-callback-'): fault=fault[len('host-callback-'):]
        if fault in ('typed-unconfirmed','typed-failed','native-failed'):
            with (Path(spec['cwd']).parent / 'terminal-launches.log').open('a',encoding='utf-8') as f: f.write(role+'\n')
        if fault.startswith('delivery-'):
            with (Path(spec['cwd']).parent / 'delivery-launches.log').open('a',encoding='utf-8') as f: f.write(role+'\n')
            if 'terra' in role:
                root=Path(spec['cwd'])
                if fault in ('delivery-positive','delivery-verify-fail','delivery-missing-output','delivery-unrelated'):
                    (root/'src/lib.rs').write_text('pub fn value() -> u8 { 2 }\n')
                if fault in ('delivery-positive','delivery-verify-fail'):
                    (root/'src/deliverable.rs').write_text('pub fn answer() -> u8 { 42 }\n')
                    (root/'src/lib.rs').write_text('#[cfg(test)] mod deliverable;\n#[test] fn delivered() { assert_eq!(deliverable::answer(), 42); }\n')
                if fault=='delivery-outside': (root/'untracked-outside.txt').write_text('outside')
                if fault=='delivery-file-delete': (root/'src/deliverable.rs').unlink()
                if fault in ('delivery-file-is-dir','delivery-ignored-dir'): (root/'src/lib.rs').write_text('pub fn value() -> u8 { 2 }\n')
                if fault in ('delivery-existing','delivery-dir','delivery-dir-delete','delivery-empty','delivery-wrong','delivery-other'):
                    if fault=='delivery-existing': (root/'src/deliverable.rs').write_text('pub fn answer() -> u8 { 42 }\n')
                    if fault=='delivery-dir': (root/'src/items/new.rs').write_text('pub fn value() {}\n')
                    if fault=='delivery-dir-delete': (root/'src/items/remove.rs').unlink()
                    if fault in ('delivery-empty','delivery-wrong','delivery-other'): (root/'src/lib.rs').write_text('pub fn value() -> u8 { 2 }\n')
        if 'astra' in role and params.get('outputSchema'):
            package=json.loads(text.split('\nPackage evidence: ',1)[1])
            receipt=Path(package['verification_receipt']['path'])
            if fault=='missing-receipt': receipt.unlink()
            if fault=='tampered-receipt': receipt.write_text('{}')
            if fault=='stale-review-source': Path(spec['cwd'],'src/lib.rs').write_text('changed during review')
        if not (fault=='missing-effort' and 'terra' in role):
            event('thread/settings/updated',tid,turnId=turn,effort=params.get('effort'))
        signal=option('--signal')
        if signal: Path(signal).write_text(tid+' '+turn)
        if fault=='crash-worker' and 'terra' in role: os._exit(0)
        if fault=='interrupt-worker' and 'terra' in role: return
        if 'terra' in role and 'ROUND 1\n' in text: time.sleep(float(option('--hold-s','0')))
        issues=['issue-a','issue-b','issue-c','issue-d'] if 'ROUND 5\n' in text else []
        if fault=='test-findings' and 'astra' in role: issues=['A required behavior is missing']
        if params.get('outputSchema') and 'astra' not in role:
            binding = params['outputSchema']['properties']
            final = json.dumps({'outcome':{'blocked-worker':'blocked','typed-unconfirmed':'unconfirmed','typed-failed':'failed'}.get(fault,'awaiting_core'),
                'operation_id':binding['operation_id']['enum'][0], 'task':binding['task']['enum'][0],
                'round':binding['round']['enum'][0], 'evidence':'Fixture stage receipt',
                'blocking_cause':'fixture missing requirement' if fault=='blocked-worker' else '',
                'next_action':'resolve blocker' if fault=='blocked-worker' else 'next Rust stage'})
            if fault in ('missing-result','delivery-native'): final='Stage completed without required typed result'
            if fault=='truncated-result': final=final[:len(final)//2]
            if fault=='wrong-identity':
                value=json.loads(final); value['operation_id']='foreign-operation'; final=json.dumps(value)
        else:
            final=json.dumps({'issues':issues,'severity':'normal','evidence':'Protocol fixture; actual Rust build/package runs in the host, not a real model assessment.'}) if params.get('outputSchema') else 'Fixture observed stage completion'
        complete(tid,turn,final,status='failed' if fault=='native-failed' else 'completed')
        return
    if scenario.startswith('attached-'):
        spec=threads[tid]
        names=[t['name'] for t in spec.get('dynamicTools',[])]
        signal=Path(option('--signal'))
        if 'chronos_codex_run' in names:
            assert spec['sandbox']=='read-only'
            assert 'mode=change' in spec['developerInstructions']
            status=tool(tid,turn,'orch',{'project':'demo','args':['status']})
            assert status['success'], status
            if scenario in ('attached-model','attached-late'):
                signal.write_text('dispatcher-running')
                return
            if scenario=='attached-nested':
                call(tid,turn,'Agent',{'subagent_type':'chronos:memory-keeper','prompt':'Own memory in dispatcher subtree','project':'demo'})
                complete(tid,turn,'PRIVATE_WORKER_SENTINEL'*2000)
                return
            if scenario=='attached-bash':
                call(tid,turn,'Bash',{'command':'git status --porcelain','timeout':120000})
                complete(tid,turn,'PRIVATE_WORKER_SENTINEL')
                return
            if scenario=='attached-smoke':
                checked=call(tid,turn,'Bash',{'command':'git status --porcelain','timeout':10000})
                assert checked['success'], checked
                checked=call(tid,turn,'Grep',{'pattern':'result.txt','path':str(Path(option('--project'))/'verify_smoke.py')})
                assert checked['success'], checked
            store=Path(option('--home'))/'orchestrate'/'demo'
            brief=store/'briefs'/'smoke.md'
            text='GOAL Write result.txt containing CHRONOS_SMOKE_OK, edit nothing else.\nSCOPE result.txt\nVERIFY '+option('--verify')+'\n'
            assert call(tid,turn,'Write',{'file_path':str(brief),'content':text})['success']
            assert tool(tid,turn,'orch',{'project':'demo','args':['unit','add','smoke','--track','build','--brief',str(brief)]})['success']
            result=tool(tid,turn,'codex_run',{'project':'demo','unit':'smoke','role':'sol','cwd':option('--project')})
            assert result['success'] or scenario=='attached-verify', result
            if scenario=='attached-verify':
                # The killed verifier can return before the model interrupt.
                # Keep this model pending to test interrupt delivery; attached-late
                # separately covers completion racing the cancellation.
                return
            complete(tid,turn,'PRIVATE_WORKER_SENTINEL'*2000)
        elif 'chronos_memory_record' in names:
            signal.write_text('nested-memory-running')
            # Intentionally no completion until interrupt.
        else:
            assert 'sol' in spec['model']
            assert spec['sandbox']=='workspace-write'
            Path(option('--project')).joinpath('result.txt').write_text('CHRONOS_SMOKE_OK')
            complete(tid,turn,'PRIVATE_SOL_SENTINEL')
        return
'''
source = source.replace(anchor, scenario + anchor)
source = source.replace("event('turn/completed',tid,turn={'id':turn,'status':'interrupted','error':None})", "event('turn/completed',tid,turn={'id':turn,'status':'completed' if scenario=='attached-late' else 'interrupted','error':None})")
source=source.replace("def run_turn(tid,turn,params):", "def guarded_turn(tid,turn,params):\n    try: run_turn(tid,turn,params)\n    except Exception as e: complete(tid,turn,'fixture failure: '+repr(e),status='failed')\n\ndef run_turn(tid,turn,params):")
source=source.replace('threading.Thread(target=run_turn,','threading.Thread(target=guarded_turn,')
exec(compile(source, 'fake_dispatcher_protocol.py', 'exec'))
