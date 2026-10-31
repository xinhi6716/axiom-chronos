from pathlib import Path
source = (Path(__file__).parent.parent / 'fakes' / 'fake_codex_session.py').read_text(encoding='utf-8')
anchor = "    elif method in ('thread/start','thread/resume'):"
assert source.count(anchor) == 1
source = source.replace(anchor, "    elif method=='skills/list': reply(msg,{'data':[{'cwd':params['cwds'][0],'errors':[],'skills':[]}]})\n" + anchor)
# Idle-wake regression scenarios stay local to the session harness.
anchor = "    if core and scenario.startswith('idle-state'):"
assert source.count(anchor) == 1
source = source.replace(anchor, """    if core and scenario.startswith('idle-fix'):
        n = turns[tid]
        if n == 1:
            assert tool(tid,turn,'goal',{'mode':'change'})['success']
            if 'dispatcher' in scenario:
                call(tid,turn,'Agent',{'subagent_type':'chronos:dispatcher','prompt':'exercise producer completion'})
            Path(record + '.boundary').touch()
            if scenario == 'idle-fix-steered':
                deadline = time.monotonic() + 10
                while not steered.get(tid):
                    if time.monotonic() > deadline: os._exit(6)
                    time.sleep(.01)
            complete(tid,turn,'producer returned without advancing work')
        else:
            if scenario == 'idle-fix-steered':
                assert tool(tid,turn,'goal',{'classify':'amend'})['success']
            else:
                assert 'Rust idle liveness failure' in json.dumps(params), params
            if scenario != 'idle-fix-ignored':
                assert tool(tid,turn,'goal',{'status':'blocked','note':'stable liveness failure delivered to core'})['success']
            complete(tid,turn)
        return
    if not core and scenario == 'idle-fix-dispatcher-failed':
        complete(tid,turn,status='failed',error={'message':'dispatcher crashed'})
        return
""" + anchor)
anchor = "    if core and scenario.startswith('idle-state'):"
source = source.replace(anchor, """    if scenario.startswith('dispatcher-boundary'):
        if core:
            assert tool(tid,turn,'goal',{'mode':'change'})['success']
            result = call(tid,turn,'Agent',{'subagent_type':'chronos:dispatcher','prompt':'boundary regression'})
            Path(record + '.callback').write_text(json.dumps(result), encoding='utf-8')
            complete(tid,turn,'dispatcher reply received')
            Path(record + '.completed-1').touch()
        else:
            Path(record + '.dispatcher-ready').touch()
            deadline = time.monotonic() + 10
            while not Path(record + '.release').exists():
                if time.monotonic() > deadline: os._exit(7)
                time.sleep(.01)
            complete(tid,turn)
            Path(record + '.dispatcher-completed').touch()
        return
""" + anchor)
exec(compile(source, 'fake_codex_session.py', 'exec'))
