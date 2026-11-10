"""Stand-in for a council member: `claude -p` for Fable (it sees --strict-mcp-config), `agy` for Argon.

--fake-log=<path> appends one JSON line per call. --fake-scenario=<a,b,...> scripts the call:
quota, crash, garbage, wrong-shape, no-usage, unpriced, r1-near-cap, r1-over-cap, r2-over-cap,
r2-garbage, r2-quota, disagree, disagree-bare, hang, shape-top, shape-object, huge, argon-tokens.
"""
import json
import os
import re
import sys
import time

sys.stdin.reconfigure(encoding='utf-8')
sys.stdout.reconfigure(encoding='utf-8')


def option(name, default=''):
    prefix = f'--fake-{name}='
    return next((arg[len(prefix):] for arg in sys.argv if arg.startswith(prefix)), default)


scenario = set(filter(None, option('scenario').split(',')))
argv = [arg for arg in sys.argv[1:] if not arg.startswith('--fake-')]
is_fable = '--strict-mcp-config' in argv
role = 'fable' if is_fable else 'argon'
prompt = sys.stdin.read() if is_fable else argv[-1]
round2 = 'CROSS-EXAMINATION' in prompt
log = option('log')
if log:
    keep = {k: os.environ.get(k) for k in ('AXIOM_ADVANCED_RESEARCH', 'CLAUDE_CODE_PROMPT_CACHE_TTL', 'CLAUDECODE')}
    # Two members start at once and an append is not atomic on Windows.
    deadline = time.monotonic() + 10
    while True:
        try:
            lock = os.open(log + '.lock', os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            break
        except (FileExistsError, PermissionError):
            if time.monotonic() > deadline:
                raise
            time.sleep(0.01)
    try:
        with open(log, 'a', encoding='utf-8') as f:
            f.write(json.dumps({'role': role, 'round': 2 if round2 else 1, 'pid': os.getpid(), 'argv': argv, 'prompt': prompt, 'env': keep, 'cwd': os.getcwd()}, ensure_ascii=False) + '\n')
    finally:
        os.close(lock)
        os.remove(log + '.lock')


def position():
    text = 'x' * 5000 if 'huge' in scenario else 'tighten the check'
    count = 14 if 'huge' in scenario else 2
    return {
        'summary': f'{role} summary',
        'coverage': {'read': ['src/mod.py'], 'not_read': []},
        'changes': [{'id': f'{role}{n}', 'area': 'design', 'location': f'src/{role}{n}.py', 'evidence': f'{role} SECRET-RAW evidence {n}',
                     'problem': f'{role} problem {n} {text}', 'change': f'claude and fable fix {n}', 'verify': 'run tests'} for n in range(count)],
        'out_of_scope': [],
    }


def stances():
    ids = sorted(set(re.findall(r'^(C\d+) \[Position', prompt, flags=re.M)))
    stance, evidence = 'agree', ''
    if 'disagree' in scenario:
        stance, evidence = 'disagree', 'src/mod.py:12 contradicts this claim'
    if 'disagree-bare' in scenario:
        stance, evidence = 'disagree', 'no'
    return {'stances': [{'claim': i, 'stance': stance, 'evidence': evidence} for i in ids]}


answer = stances() if round2 else position()
if 'DIFF REVIEW' in prompt:
    answer = {'verdict': 'revise', 'findings': [{'severity': 'major', 'location': 'src/a.py:1', 'reason': 'agy reviewed this diff'}]} if 'BLOCKME' in prompt else {'verdict': 'pass', 'findings': []}
if 'wrong-shape' in scenario:
    answer = {'wrong': 'shape'}
garbage = 'garbage' in scenario or (round2 and 'r2-garbage' in scenario)
out_of_quota = 'quota' in scenario or (round2 and 'r2-quota' in scenario)

if 'hang' in scenario:
    time.sleep(60)
if 'crash' in scenario:
    print('boom', file=sys.stderr)
    sys.exit(3)

if not is_fable:
    if out_of_quota:
        print('Error: RESOURCE_EXHAUSTED: You have exceeded your quota', file=sys.stderr)
        sys.exit(1)
    if garbage:
        print('this is not json')
        sys.exit(0)
    tokens = 900_000 if 'argon-tokens' in scenario else 1234
    if 'shape-top' in scenario:
        print(json.dumps(answer, ensure_ascii=False))
    elif 'shape-object' in scenario:
        print(json.dumps({'output': answer, 'usageMetadata': {'promptTokenCount': 1000, 'candidatesTokenCount': 234}}, ensure_ascii=False))
    else:
        print(json.dumps({'result': json.dumps(answer, ensure_ascii=False), 'usage': {'total_tokens': tokens}}, ensure_ascii=False))
    sys.exit(0)

print('unrecognized_model: claude-fable-5-2', file=sys.stderr, flush=True)


def emit(event):
    print(json.dumps(event, ensure_ascii=False), flush=True)


emit({'type': 'system', 'subtype': 'init', 'session_id': 'member'})
if 'r1-over-cap' in scenario and not round2 or 'r2-over-cap' in scenario and round2:
    # The stream's output count stays tiny; only the content shows how much was generated.
    emit({'type': 'assistant', 'message': {'id': 'big', 'model': 'claude-fable-5-2', 'content': [{'type': 'text', 'text': 'x' * 600_000}], 'usage': {'input_tokens': 10, 'output_tokens': 1}}})
    time.sleep(30)
    sys.exit(0)
usage = {'input_tokens': 1000, 'cache_read_input_tokens': 2000, 'cache_creation_input_tokens': 4000, 'output_tokens': 500}
model = 'claude-experimental-x' if 'unpriced' in scenario else 'claude-fable-5-2'
message_id = 'r2' if round2 else 'r1'
if 'no-usage' not in scenario:
    # Stream-json repeats a message's usage on each content block.
    for _ in range(3):
        emit({'type': 'assistant', 'message': {'id': message_id, 'model': model, 'content': [], 'usage': usage}})
if out_of_quota:
    emit({'type': 'result', 'is_error': True, 'result': "You've hit your usage limit", 'total_cost_usd': 99.0})
    sys.exit(1)
if garbage:
    emit({'type': 'result', 'is_error': False, 'result': 'no json here', 'total_cost_usd': 99.0})
    sys.exit(0)
closing = {'type': 'result', 'is_error': False, 'result': json.dumps(answer, ensure_ascii=False), 'total_cost_usd': 99.0}
if 'r1-near-cap' in scenario and not round2:
    # The true total arrives only here, as it does from stream-json.
    closing['usage'] = {'input_tokens': 1000, 'output_tokens': 115_000}
emit(closing)
