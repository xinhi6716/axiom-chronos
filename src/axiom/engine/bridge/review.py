"""Cross-family code review: a Gemini model checks Codex-written files against the contract they were built to."""
from pathlib import Path

import agy_print
from procs import BridgeError, require_directory

MAX_FILE_BYTES = 60_000
# One 134 KB review left Gemini 4 Argon High with 33.6k thinking tokens and no answer; review a module at a time.
MAX_TOTAL_BYTES = 60_000
CHECKS = ('contract_mismatch', 'vacuous_check', 'untested', 'platform')

SCHEMA = {
    'type': 'object',
    'required': ['verdict', 'findings'],
    'properties': {
        'verdict': {'type': 'string', 'enum': ['pass', 'fail']},
        'findings': {'type': 'array', 'items': {
            'type': 'object',
            'required': ['check', 'severity', 'location', 'evidence', 'fix'],
            'properties': {
                'check': {'type': 'string', 'enum': list(CHECKS)},
                'severity': {'type': 'string', 'enum': ['blocker', 'major', 'minor']},
                'location': {'type': 'string'},
                'evidence': {'type': 'string'},
                'fix': {'type': 'string'},
            },
        }},
    },
}

INSTRUCTION = """Every file to review is included in this message; answer from it directly.

Review the files below against the contract. Report every:
- contract_mismatch: a name, signature, data shape, call to another component or invariant that differs from the contract;
- vacuous_check: a test, assertion or formal invariant that cannot fail, for example a TLA+ variable that no action ever sets to the value the invariant forbids, a TLA+ invariant that restates the guard of the only action able to break it, or a test whose assertion holds whatever the code does;
- untested: a promise in the contract that no test exercises;
- platform: behaviour that depends on the operating system, path separators, or a fixed inventory of files.
For each, give the severity (blocker, major or minor), the file and line or name, the evidence quoted from the code, and the fix. The verdict is fail when any finding is a blocker or major."""


def review(project_dir, paths, contract, model, timeout_s=900, command=None):
    """Files are read here and sent inline, so the reviewer runs in an empty directory and cannot change the project."""
    require_directory(project_dir)
    if not contract or not contract.strip():
        raise BridgeError('invalid_argument', 'contract is empty; review against what the code was built to do')
    if not paths:
        raise BridgeError('invalid_argument', 'paths is empty')
    root = Path(project_dir).resolve()
    blocks, total = [], 0
    for rel in paths:
        path = (root / rel).resolve()
        if root not in path.parents or not path.is_file():
            raise BridgeError('invalid_argument', f'not a file inside project_dir: {rel}')
        data = path.read_bytes()
        if len(data) > MAX_FILE_BYTES:
            raise BridgeError('invalid_argument', f'{rel} is {len(data)} bytes; split the review (limit {MAX_FILE_BYTES})')
        total += len(data)
        blocks.append(f'=== FILE {Path(rel).as_posix()}\n{data.decode("utf-8", errors="replace")}')
    if total > MAX_TOTAL_BYTES:
        raise BridgeError('invalid_argument', f'{total} bytes across {len(paths)} files; split the review (limit {MAX_TOTAL_BYTES})')
    prompt = f'{INSTRUCTION}\n\n=== CONTRACT\n{contract}\n\n' + '\n\n'.join(blocks)
    for _ in range(2):
        result = agy_print.run(command, prompt, None, model, json_schema=SCHEMA, timeout_s=timeout_s)
        # An empty answer is a transient refusal or budget overrun, never a pass; one retry, then report it.
        if (result.get('error') or {}).get('kind') != 'empty_response':
            break
    if result['status'] != 'completed':
        return result
    output = result.get('structured_output')
    if not isinstance(output, dict) or not isinstance(output.get('findings'), list):
        return {'status': 'failed', 'verdict': None, 'findings': [], 'reviewer': model,
                'bytes_reviewed': total, 'usage': result.get('usage'),
                'error': {'kind': 'no_review_output', 'message': 'Review output is missing a findings list'}}
    findings = output['findings']
    serious = [f for f in findings if f.get('severity') in ('blocker', 'major')]
    # A reviewer can fail code but never pass code it reported serious problems in.
    return {'status': 'completed', 'verdict': 'fail' if serious or output.get('verdict') == 'fail' else 'pass', 'findings': findings,
            'reviewer': model, 'bytes_reviewed': total, 'usage': result.get('usage'), 'error': None}
