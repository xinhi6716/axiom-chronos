"""Stand-in for `wsl.exe --cd DIR -e sh -c SCRIPT axiom-linux ARGS...`: runs `python ARGS` in DIR on this host."""
import os
import subprocess
import sys

argv = sys.argv[1:]
if (len(argv) < 8 or argv[0] != '--cd' or argv[2:7] != [
        '-e', 'sh', '-c', 'exec "$HOME/.axiom/linux-venv/bin/python" "$@"',
        'axiom-linux']):
    sys.stderr.write('unexpected wsl arguments\n')
    sys.exit(2)
cwd = argv[1]
args = argv[7:]
capture = os.path.join(cwd, '.fake_wsl_capture_cd')
if os.path.exists(capture):
    with open(capture, 'w', encoding='utf-8') as f:
        f.write(cwd)
if os.path.exists(os.path.join(cwd, 'FAKE_WSL_NO_VENV')):
    sys.stderr.write('sh: 1: exec: /home/u/.axiom/linux-venv/bin/python: not found\n')
    sys.exit(127)
raise SystemExit(subprocess.run([sys.executable] + args, cwd=cwd).returncode)
