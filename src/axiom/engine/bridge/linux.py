"""Run a Python command in the project on Linux: Windows-only runs skip or mask POSIX-only tests."""
import os
import subprocess
from pathlib import Path

from procs import BridgeError, require_directory, resolve_command

# Resolved inside WSL, so the Windows side never needs the Linux user's home directory.
PYTHON_IN_WSL = '$HOME/.axiom/linux-venv/bin/python'
TAIL = 4000
ON_WINDOWS = os.name == 'nt'


def command_for(project_dir, args):
    """Through WSL on Windows; a POSIX host already runs its tests on POSIX."""
    if not ON_WINDOWS and not os.environ.get('AXIOM_WSL_CMD'):
        return None
    try:
        wsl = resolve_command('AXIOM_WSL_CMD', 'wsl.exe')
    except BridgeError:
        return None
    # Arguments travel as "$@", so nothing in them is parsed by a shell.
    return wsl + ['--cd', str(project_dir), '-e', 'sh', '-c', f'exec "{PYTHON_IN_WSL}" "$@"', 'axiom-linux'] + list(args)


def run(project_dir, args, timeout_s=1800):
    require_directory(project_dir)
    if not isinstance(args, list) or not args or not all(isinstance(a, str) for a in args):
        raise BridgeError('invalid_argument', 'args must be a non-empty array of strings passed to python, e.g. ["-m", "unittest"]')
    argv = command_for(Path(project_dir).resolve(), args)
    if argv is None:
        reason = 'WSL is not installed' if ON_WINDOWS else 'this host is not Windows; run the command locally'
        return {'status': 'unavailable', 'reason': reason, 'error': None}
    try:
        done = subprocess.run(argv, cwd=project_dir, stdin=subprocess.DEVNULL, capture_output=True,
                              text=True, encoding='utf-8', errors='replace', timeout=timeout_s)
    except subprocess.TimeoutExpired:
        raise BridgeError('timeout', f'linux run exceeded {timeout_s}s') from None
    # wsl.exe writes some of its own messages as UTF-16, which shows up here as NUL-padded text.
    output = (done.stdout + done.stderr).replace(chr(0), '')
    if done.returncode == 127:
        return {'status': 'unavailable', 'reason': 'no ~/.axiom/linux-venv in WSL; see axiom doctor', 'output_tail': output[-TAIL:], 'error': None}
    return {'status': 'completed' if done.returncode == 0 else 'failed', 'exit_code': done.returncode, 'output_tail': output[-TAIL:],
            'error': None if done.returncode == 0 else {'kind': 'linux_command_failed', 'message': output[-800:]}}
