"""Cross-family code review: a Gemini model checks Codex-written files against the contract they were built to."""
from pathlib import Path

import agy_print
from procs import BridgeError, require_directory

MAX_FILE_BYTES = 80_000
MAX_TOTAL_BYTES = 400_000
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

INSTRUCTION = """Review the files below against the contract. Report every:
- contract_mismatch: a name, signature, data shape, call to another component or invariant that differs from the contract;
- vacuous_check: a test, assertion or formal invariant that cannot fail, for example a TLA+ variable that no action ever sets to the value the invariant forbids, or a test whose assertion holds whatever the code does;
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
    result = agy_print.run(command, prompt, None, model, json_schema=SCHEMA, timeout_s=timeout_s)
    if result['status'] != 'completed':
        return result
    findings = (result.get('structured_output') or {}).get('findings') or []
    serious = [f for f in findings if f.get('severity') in ('blocker', 'major')]
    # The verdict follows the findings, so a reviewer cannot pass code it reported serious problems in.
    return {'status': 'completed', 'verdict': 'fail' if serious else 'pass', 'findings': findings,
            'reviewer': model, 'bytes_reviewed': total, 'usage': result.get('usage'), 'error': None}
