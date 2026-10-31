"""Codex app-server session fake: exercises actual callback RPC, independent threads and steering."""
import json, os, sys, threading, time, subprocess
from pathlib import Path
sys.stdin.reconfigure(encoding='utf-8')
sys.stdout.reconfigure(encoding='utf-8')
if '--version' in sys.argv:
    print('codex-cli session-fake')
    sys.exit(0)
def option(name, default=None):
    return sys.argv[sys.argv.index(name)+1] if name in sys.argv else default
scenario = option('--scenario', os.getenv('FAKE_CODEX_SCENARIO', 'ok'))
record = option('--record', os.getenv('FAKE_CODEX_RECORD'))
lock = threading.RLock()
requests, responses, threads, waiters, steered, turns = [], [], {}, {}, {}, {}
serial = 10000

def persist():
    if record:
        Path(record).write_text(json.dumps({'argv':sys.argv[1:], 'requests':requests, 'responses':responses, 'cwd':os.getcwd()},ensure_ascii=False),encoding='utf-8')
def send(v):
    with lock:
        print(json.dumps(v,ensure_ascii=False),flush=True)
def reply(msg,result): send({'id':msg['id'],'result':result})
def event(method, tid, **params): send({'method':method,'params':dict(threadId=tid,**params)})
def complete(tid,turn,text='完成中文',status='completed',error=None):
    if (tid,turn) in cancelled: return
    event('item/completed',tid,turnId=turn,item={'type':'agentMessage','id':'m','text':text})
    event('thread/tokenUsage/updated',tid,tokenUsage={'total':{'inputTokens':100,'cachedInputTokens':20,'outputTokens':10}})
    event('turn/completed',tid,turn={'id':turn,'status':status,'error':error})
def call(tid,turn,tool,args):
    global serial
    with lock:
        serial+=1
        id=serial
        done=threading.Event()
        waiters[id]=(done,None)
    send({'id':id,'method':'item/tool/call','params':{'threadId':tid,'turnId':turn,'callId':str(id),'tool':tool,'arguments':args}})
    # The host owns callback deadlines; a valid nested engineering operation can
    # outlive the model turn's budget.  Wait for its reply or turn interruption,
    # rather than silently killing this daemon thread after an arbitrary timeout.
    while not done.wait(.05):
        if (tid,turn) in cancelled:
            with lock:
                waiters.pop(id, None)
            return {'success':False,'contentItems':[{'type':'inputText','text':'turn interrupted'}]}
    with lock:
        _,result=waiters.pop(id)
    return result
PREFIX='chronos_'
def tool(tid,turn,name,args): return call(tid,turn,PREFIX+name,args)
def finish(tid,turn):
    tool(tid,turn,'goal',{'session':'forged-root','status':'done'})
    complete(tid,turn)
cancelled=set()
children=[]
def run_turn(tid,turn,params):
    role=threads[tid].get('model','')
    core=not threads[tid].get('ephemeral',False) and 'You are Chronos core' in threads[tid].get('developerInstructions','')
    if core and scenario.startswith('idle-state'):
        n = turns[tid]
        marker = Path(record + '.completed-' + str(n))
        if n == 1:
            assert tool(tid,turn,'goal',{'mode':'change'})['success']
            # Tests control the turn/wait boundary without racing a timer.
            if scenario in ('idle-state-boundary', 'idle-state-callback'):
                Path(record + '.boundary').touch()
                deadline = time.monotonic() + 10
                while not Path(record + '.release').exists():
                    if time.monotonic() > deadline: os._exit(4)
                    time.sleep(.01)
            if scenario == 'idle-state-callback':
                result = call(tid,turn,'Agent',{'subagent_type':'chronos:dispatcher','prompt':'return durable notices'})
                assert result['success'], result
        else:
            g = json.loads(Path(option('--home'), 'goals', tid + '.json').read_text(encoding='utf-8'))
            if g.get('interjection'):
                Path(record + '.interjection').write_text(json.dumps(g), encoding='utf-8')
                assert tool(tid,turn,'goal',{'classify':'amend'})['success']
                assert tool(tid,turn,'goal',{'status':'blocked','note':'test finished'})['success']
        complete(tid,turn,'waiting for durable change')
        marker.touch()
        if n == 1:
            # Bound failures in idle-loop tests even if the host misses a wake.
            def watchdog():
                time.sleep(15)
                os._exit(5)
            threading.Thread(target=watchdog,daemon=True).start()
        return
    if core and scenario=='idle-wake':
        if turns[tid] == 1:
            assert tool(tid,turn,'goal',{'mode':'change'})['success']
            complete(tid,turn,'waiting')
        else:
            assert tool(tid,turn,'goal',{'classify':'amend'})['success']
            tool(tid,turn,'goal',{'status':'blocked','note':'test finished'})
            complete(tid,turn)
        return
    if not core and 'terra' in role and threads[tid].get('dynamicTools'):
        if scenario in ('planner','planner-invalid','planner-limit'):
            index=int(params['input'][0]['text'])
            result=call(tid,turn,'Agent',{'subagent_type':'chronos:astra','prompt':'plan independently','project':'demo','cwd':option('--planner-cwd',option('--project')) if index else option('--project'),'schema_path':option('--schema'),'save_to':option('--output'),'web_search':bool(index)})
            complete(tid,turn,json.dumps(result))
            return
        if scenario == 'agents':
            call(tid,turn,'Agent',{'subagent_type':'chronos:reviewer','prompt':'review independently'})
            call(tid,turn,'Agent',{'subagent_type':'chronos:memory-keeper','prompt':'remember'})
            complete(tid,turn,'routed specialists')
            return
        if scenario == 'authority':
            call(tid,turn,'Agent',{'subagent_type':'chronos:content','prompt':'write document','output_paths':[option('--output')]})
            complete(tid,turn,'delegated content')
            return
    if not core:
        if scenario=='fallback' and 'terra' in role:
            result=call(tid,turn,'Agent',{'subagent_type':'chronos:reviewer','prompt':'review Sol writer fallback','project':'demo'})
            call(tid,turn,'Agent',{'subagent_type':'chronos:critic','prompt':'research critique'})
            complete(tid,turn,json.dumps(result))
            return
        if scenario in ('verify-stop','verify-interrupt') and 'terra' in role and threads[tid].get('dynamicTools'):
            tool(tid,turn,'codex_run',{'project':'demo','unit':'u','cwd':option('--project'),'role':'terra'})
            return
        if scenario=='cancel' and 'terra' in role:
            while (tid,turn) not in cancelled: time.sleep(.01)
            return
        if scenario=='delegated-authority' and 'terra' in role:
            results={}
            results['Write']=call(tid,turn,'Write',{'file_path':option('--output'),'content':'NO dispatcher write'})
            results['Escalate']=call(tid,turn,'Agent',{'subagent_type':'chronos:content','prompt':'NO escalation','output_paths':[option('--output')+'.other']})
            results['Content']=call(tid,turn,'Agent',{'subagent_type':'chronos:content','prompt':'describe image','output_paths':[option('--output')]})
            complete(tid,turn,json.dumps(results))
        elif scenario in ('authority','delegated-authority'):
            results={}
            event('item/started',tid,turnId=turn,item={'type':'imageView','id':'image','path':str(Path(option('--project')).joinpath('image.png'))})
            event('item/completed',tid,turnId=turn,item={'type':'imageView','id':'image','path':str(Path(option('--project')).joinpath('image.png'))})
            results['Read']=call(tid,turn,'Read',{'file_path':str(Path(option('--project')).joinpath('input.txt'))})
            results['Write']=call(tid,turn,'Write',{'file_path':option('--output'),'content':'explicit document artifact'})
            results['Escape']=call(tid,turn,'Write',{'file_path':option('--output')+'.other','content':'NO'})
            results['Nest']=call(tid,turn,'Agent',{'subagent_type':'chronos:dispatcher','prompt':'unauthorized'})
            complete(tid,turn,json.dumps(results))
        elif 'outputSchema' in params:
            assert call(tid,turn,'Read',{'file_path':'input.txt'})['success']
            complete(tid,turn,json.dumps({'coverage':{'read':['input.txt'],'not_read':[]},'plan':'independent'}))
        else: complete(tid,turn,'child report')
        return
    if scenario=='crash':
        time.sleep(.05)
        os._exit(3)
    if scenario in ('interrupt','hang'):
        if scenario=='interrupt':
            child=subprocess.Popen([sys.executable,'-c','import time;time.sleep(120)'])
            children.append(child)
            Path(option('--pids')).write_text(f'{os.getpid()} {child.pid}')
        return
    if scenario in ('error','limit'):
        complete(tid,turn,status='failed',error={'message':'usage capacity limit' if scenario=='limit' else 'turn failed','codexErrorInfo':'usageLimitExceeded'})
        return
    if scenario=='tool-loop':
        for _ in range(100):
            if (tid,turn) in cancelled: return
            result=tool(tid,turn,'workflow',{'action':'status'})
            if not result['success']: return
        return
    if scenario=='native-loop':
        for i in range(100):
            if (tid,turn) in cancelled: return
            event('item/started',tid,turnId=turn,item={'type':'imageView','id':str(i),'path':'image.png'})
            time.sleep(.01)
        return
    if scenario=='forever':
        complete(tid,turn,'still active')
        return
    if turns[tid]==1:
        tool(tid,turn,'goal',{'mode':'change' if scenario in ('agents','cancel','delegated-authority','fallback','verify-stop','verify-interrupt') else 'answer'})
    if scenario in ('verify-stop','verify-interrupt'):
        threading.Thread(target=lambda:call(tid,turn,'Agent',{'subagent_type':'chronos:dispatcher','prompt':'run verifier'}),daemon=True).start()
        if scenario=='verify-stop':
            deadline=time.monotonic()+10
            while not steered.get(tid) and time.monotonic()<deadline: time.sleep(.01)
            if not steered.get(tid):
                complete(tid,turn,status='failed',error={'message':'verifier stop was never steered'})
                return
            tool(tid,turn,'goal',{'classify':'stop'})
        else:
            deadline=time.monotonic()+15
            while (tid,turn) not in cancelled and time.monotonic()<deadline: time.sleep(.01)
            complete(tid,turn,status='failed',error={'message':'verifier interrupt timed out'})
        return
    if scenario=='fallback':
        call(tid,turn,'Agent',{'subagent_type':'chronos:dispatcher','prompt':'dispatch reviewer'})
        tool(tid,turn,'goal',{'status':'blocked','note':'fallback probe complete'})
        complete(tid,turn)
        return
    if scenario=='blocked':
        tool(tid,turn,'goal',{'status':'blocked','note':'need input'})
        complete(tid,turn)
        return
    if scenario=='continue' and turns[tid]==1:
        complete(tid,turn,'not yet complete')
        return
    if scenario in ('interject','forward','cancel','stale') and (scenario!='stale' or turns[tid]==1):
        expected=2 if scenario=='forward' else 1
        deadline=time.monotonic()+10
        if scenario=='cancel':
            # The child call remains outstanding while the user stops the root.
            threading.Thread(target=lambda:call(tid,turn,'Agent',{'subagent_type':'chronos:dispatcher','prompt':'wait'}),daemon=True).start()
        while len(steered.get(tid,[]))<expected and time.monotonic()<deadline: time.sleep(.01)
        if scenario=='stale':
            complete(tid,turn,'prior turn completed')
            return
        assert not tool(tid,turn,'goal',{'status':'done'})['success']
        result=tool(tid,turn,'goal',{'classify':'stop' if scenario=='cancel' else 'amend'})
        if scenario=='cancel': return
        event('item/completed',tid,turnId=turn,item={'type':'agentMessage','text':' '.join(steered.get(tid,[]))})
    if scenario=='stale' and turns[tid]>1:
        tool(tid,turn,'goal',{'classify':'amend'})
    if scenario=='agents':
        # Direct dispatcher requires change mode.
        call(tid,turn,'Agent',{'subagent_type':'chronos:dispatcher','prompt':'dispatch directly'})
    if scenario in ('planner','planner-invalid','planner-limit'):
        state=tool(tid,turn,'workflow',{'action':'status'})
        current=json.loads(state['contentItems'][0]['text'])['workflow']
        if current['phase']=='intent_review':
            tool(tid,turn,'workflow',{'action':'verdict','revision':current['revision'],'decision':'reject','reason':'resumed intent requires a revised proposal'})
        for i in range(1 if scenario=='planner-limit' else 2):
            result=call(tid,turn,'Agent',{'subagent_type':'chronos:dispatcher','prompt':str(i),'project':'demo'})
            state=tool(tid,turn,'workflow',{'action':'status'})
            current=json.loads(state['contentItems'][0]['text'])['workflow']
            if current['phase']=='intent_review' and scenario!='planner-limit':
                decision={'action':'verdict','revision':current['revision'],'decision':'reject' if i==0 else 'pass'}
                if i==0: decision['reason']='first proposal does not match requested scope'
                tool(tid,turn,'workflow',decision)
    if scenario=='planner-limit':
        complete(tid,turn,status='failed',error={'message':'usage capacity limit'})
        return
    if scenario=='authority':
        call(tid,turn,'Write',{'file_path':option('--output')+'.core','content':'NO'})
        call(tid,turn,PREFIX+'codex_run',{'role':'terra','cwd':option('--project'),'brief':'NO'})
        call(tid,turn,'Agent',{'subagent_type':'chronos:dispatcher','prompt':'write document','output_paths':[option('--output')]})
    if scenario=='delegated-authority':
        call(tid,turn,'Agent',{'subagent_type':'chronos:dispatcher','prompt':'organize documents','output_paths':[option('--output')]})
        tool(tid,turn,'goal',{'status':'blocked','note':'authority probe complete'})
        complete(tid,turn)
        return
    if scenario=='agents':
        tool(tid,turn,'goal',{'status':'blocked','note':'agent probe complete'})
        complete(tid,turn)
        return
    if scenario=='bad-goal':
        Path(option('--home')).joinpath('goals',tid+'.json').write_text('{')
        complete(tid,turn)
        return
    finish(tid,turn)

for line in sys.stdin:
    msg=json.loads(line)
    method=msg.get('method'); params=msg.get('params',{})
    with lock:
        if method:
            requests.append(msg);persist()
        else:
            responses.append(msg);persist()
            if msg.get('id') in waiters:
                done,_=waiters[msg['id']]
                waiters[msg['id']]=(done,msg.get('result',msg.get('error')))
                done.set()
            continue
    if method=='initialize': reply(msg,{'userAgent':'fake'})
    elif method=='thread/unsubscribe': reply(msg,{'status':'unsubscribed'})
    elif method=='model/list': reply(msg,{'data':[{'id':'gpt-6-'+r} for r in ('astra','terra','sol','luna')]})
    elif method=='plugin/installed': reply(msg,{'marketplaces':[{'plugins':[{'id':'fixture@test','enabled':True}]}]})
    elif method=='config/read': reply(msg,{'config':{'mcp_servers':{'fixture':{'command':'never-start'}}}})
    elif method in ('thread/start','thread/resume'):
        if any(t['name'].startswith('mcp__') for t in params.get('dynamicTools',[])):
            send({'id':msg['id'],'error':{'code':-32600,'message':'dynamic tool name is reserved: mcp__'}})
            continue
        tid=params.get('threadId') or (f'thread-{len(threads)+1}' if '--scenario' in sys.argv else f'thread-{os.getpid()}-{len(threads)+1}')
        threads[tid]=params
        reply(msg,{'thread':{'id':tid},'model':params['model'] if scenario!='model-mismatch' else 'gpt-6-luna','sandbox':{'type':'workspaceWrite' if scenario=='unsafe-sandbox' else 'readOnly'},'approvalPolicy':'never'})
    elif method=='turn/start':
        tid=params['threadId'];turns[tid]=turns.get(tid,0)+1
        turn=f'turn-{turns[tid]}'
        reply(msg,{'turn':{'id':turn}})
        threading.Thread(target=run_turn,args=(tid,turn,params),daemon=True).start()
    elif method=='turn/steer':
        steered.setdefault(params['threadId'],[]).append(params['input'][0]['text'])
        if scenario=='stale' and turns[params['threadId']]==1:
            send({'id':msg['id'],'error':{'code':-32600,'message':'expected turn is no longer active'}})
        else: reply(msg,{'turnId':params['expectedTurnId']})
    elif method=='turn/interrupt':
        tid=params['threadId'];turn=params['turnId']
        cancelled.add((tid,turn))
        for child in children:
            if child.poll() is None: child.kill()
            child.wait(timeout=5)
        reply(msg,{})
        event('turn/completed',tid,turn={'id':turn,'status':'interrupted','error':None})
