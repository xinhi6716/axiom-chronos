import json, pathlib, subprocess, sys, time
root=pathlib.Path(sys.argv[3]).resolve(); root.mkdir(parents=True, exist_ok=True)
tag=sys.argv[1]; exe=sys.argv[2]
cwd=root/'model-fixture';cwd.mkdir(exist_ok=True)
tasks=[('clamp','fn bounded(x: i32, lo: i32, hi: i32) -> i32 { x.min(lo).max(hi) }','Return x clamped to the inclusive interval [lo,hi]. Callers guarantee lo<=hi. Preserve signature.','assert_eq!(bounded(5,0,10),5);assert_eq!(bounded(-1,0,10),0);assert_eq!(bounded(11,0,10),10);assert_eq!(bounded(0,0,0),0);'),('positive_sum','fn positive_sum(xs: &[i32]) -> i64 { xs.iter().map(|x| *x as i64).sum() }','Return sum of strictly positive inputs, using i64 accumulation. Preserve signature.','assert_eq!(positive_sum(&[-2,3,0,4]),7);assert_eq!(positive_sum(&[]),0);assert_eq!(positive_sum(&[i32::MAX,i32::MAX]),4294967294);')]
p=subprocess.Popen([exe,'bridge'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=open(root/(tag+'-model-stderr.txt'),'w'),text=True,encoding='utf-8')
results=[]
for i,(name,code,contract,checks) in enumerate(tasks,1):
 brief=f'GOAL Repair this single Rust function: {contract}\nSCOPE Only return the complete corrected function in JSON field code; no filesystem reads, tools, delegation, explanations or file edits. All needed code is provided.\nVERIFY The caller compiles and checks the returned function.\nCODE {code}'
 args=dict(cwd=str(cwd),role='terra',model='gpt-6.1-terra',effort='high',sandbox='read-only',brief=brief,timeout_s=90,output_schema=dict(type='object',properties=dict(code=dict(type='string')),required=['code'],additionalProperties=False))
 start=time.monotonic();p.stdin.write(json.dumps(dict(jsonrpc='2.0',id=i,method='tools/call',params=dict(name='codex_run',arguments=args)))+'\n');p.stdin.flush()
 msg=json.loads(p.stdout.readline());r=msg.get('result',{}).get('structuredContent',{})
 elapsed=time.monotonic()-start
 (root/f'{tag}-{name}-raw.json').write_text(json.dumps(msg,indent=2,ensure_ascii=False),encoding='utf-8')
 code=(r.get('structured_output') or {}).get('code','')
 path=cwd/f'{tag}_{name}.rs';path.write_text(code+'\nfn main(){'+checks+'}\n',encoding='utf-8')
 binary=cwd/f'{tag}_{name}.exe'
 compile=subprocess.run(['rustc','--edition=2021',str(path),'-o',str(binary)],capture_output=True,text=True)
 passed=compile.returncode==0 and subprocess.run([str(binary)],capture_output=True).returncode==0
 results.append(dict(task=name,status=r.get('status'),passed=passed,wall_s=round(elapsed,3),usage=r.get('usage'),model=r.get('model'),commands=len(r.get('commands',[])),started_mcp_servers=r.get('started_mcp_servers'),error=r.get('error')))
 print(json.dumps(results[-1]),flush=True)
p.stdin.close();p.wait(timeout=30)
(root/(tag+'-model-results.json')).write_text(json.dumps(results,indent=2),encoding='utf-8')

