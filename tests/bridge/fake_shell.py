#!/usr/bin/env python3
"""Long-lived Host shell stand-in; only used in an isolated test process."""
import os
import subprocess
import sys
import time
from pathlib import Path

assert 'git status --porcelain' in sys.argv
child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(120)'])
Path(os.environ['CHRONOS_TEST_SHELL_SIGNAL']).write_text(f'{os.getpid()} {child.pid}')
time.sleep(120)
