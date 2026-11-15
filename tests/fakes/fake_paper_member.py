"""Fake Claude and agy paper members; records exact prompts and process arguments."""
import json
import os
import sys
import time

args = sys.argv[1:]
def flag(name, default=None):
    prefix = name + '='
    return next((a[len(prefix):] for a in args if a.startswith(prefix)), default)

fable = '--strict-mcp-config' in args
stream = '--input-format' in args
if fable:
    prompt = sys.stdin.buffer.read().decode('utf-8')
elif stream:
    message = json.loads(sys.stdin.buffer.readline().decode('utf-8'))
    prompt = message['message']['content'][0]['text']
else:
    prompt = args[args.index('--print') + 1]

log = flag('--fake-log')
if log:
    with open(log, 'a', encoding='utf-8', newline='\n') as f:
        f.write(json.dumps({'argv': args, 'prompt': prompt, 'cwd': os.getcwd()}, ensure_ascii=False) + '\n')
scenario = flag('--fake-scenario', 'ok')
model = args[args.index('--model') + 1]
if scenario == 'hang':
    time.sleep(90)
if scenario == 'quota':
    print('RESOURCE_EXHAUSTED: quota exceeded', file=sys.stderr)
    sys.exit(1)
if scenario == 'wrong-model':
    if fable:
        print(json.dumps({'type': 'result', 'is_error': True, 'result': 'requested model unavailable'}), flush=True)
    else:
        print('requested model unavailable', file=sys.stderr)
    sys.exit(1)
if flag('--fake-paper-output') == '1':
    from pathlib import Path
    number = int(Path.cwd().name.split('-')[0])
    fixture = Path(__file__).resolve().parents[1] / 'fixtures' / 'paper' / 'runner' / ('pass%d.json' % number)
    answer = fixture.read_text(encoding='utf-8')
else:
    answer = json.dumps({'ok': True, 'echo_len': len(prompt)}, ensure_ascii=False)
if scenario == 'reply-wrong-model':
    model = 'wrong-model'

usage = {'input_tokens': 1000, 'output_tokens': 100}
if fable:
    if scenario != 'missing-usage':
        print(json.dumps({'type': 'assistant', 'message': {'id': 'message-1', 'model': model, 'content': [], 'usage': usage}}), flush=True)
    result = {'type': 'result', 'is_error': False, 'result': 'not json' if scenario == 'garbage' else answer}
    if scenario != 'missing-usage':
        result['usage'] = usage
    print(json.dumps(result, ensure_ascii=False), flush=True)
elif scenario == 'garbage':
    print('not json', flush=True)
elif stream:
    print(json.dumps({'type': 'result', 'result': answer, 'usage': usage, 'model': model}, ensure_ascii=False), flush=True)
else:
    print(json.dumps({'result': answer, 'usage': usage, 'model': model}, ensure_ascii=False), flush=True)
