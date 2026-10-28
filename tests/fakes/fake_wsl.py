"""Stand-in for `wsl.exe --cd DIR -e sh -c SCRIPT axiom-linux ARGS...`: runs `python ARGS` in DIR on this host."""
import os
import subprocess
import sys

argv = sys.argv[1:]
cwd = argv[argv.index('--cd') + 1]
args = argv[argv.index('axiom-linux') + 1:]
if os.environ.get('FAKE_WSL_NO_VENV'):
    sys.stderr.write('sh: 1: exec: /home/u/.axiom/linux-venv/bin/python: not found\n')
    sys.exit(127)
raise SystemExit(subprocess.run([sys.executable] + args, cwd=cwd).returncode)
