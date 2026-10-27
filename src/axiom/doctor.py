"""Check everything `axiom wish` depends on and say what to fix."""
import os
import sqlite3
import subprocess
import sys

from axiom.home import axiom_home, resolve

REQUIRED, OPTIONAL = 'required', 'optional'


def version(command):
    try:
        done = subprocess.run(command + ['--version'], stdin=subprocess.DEVNULL, capture_output=True, text=True,
                              encoding='utf-8', errors='replace', timeout=30)
    except (OSError, subprocess.TimeoutExpired) as exc:
        return None, str(exc)
    text = (done.stdout or done.stderr).strip().splitlines()
    return (text[-1] if text else '') if done.returncode == 0 else None, (done.stderr or done.stdout).strip()[-200:]


def tool(label, env_name, executable, need, purpose, fix):
    command = resolve(env_name, executable)
    if command is None:
        return label, False, need, f'not on PATH. {fix}'
    found, detail = version(command)
    return (label, True, need, f'{found} ({purpose})') if found is not None else (label, False, need, f'{command[0]} failed: {detail}')


def windows_shell():
    """Codex runs worker commands through pwsh when one is on PATH, and its sandbox cannot start Store (MSIX) apps."""
    command = resolve('AXIOM_PWSH_CMD', 'pwsh')
    if command is None:
        return 'pwsh', True, OPTIONAL, 'not found; Codex falls back to Windows PowerShell, which its sandbox can start'
    if 'WindowsApps' in command[0]:
        return 'pwsh', False, REQUIRED, ('Store build; the Codex sandbox cannot start it, so workers cannot run tests. '
                                         'Remove it, then: winget install --id Microsoft.PowerShell --exact --installer-type wix')
    return 'pwsh', True, OPTIONAL, command[0]


def sqlite_trigram():
    db = sqlite3.connect(':memory:')
    try:
        db.execute("CREATE VIRTUAL TABLE t USING fts5(x, tokenize='trigram')")
        return 'sqlite fts5', True, REQUIRED, f'SQLite {sqlite3.sqlite_version} with trigram (memory search)'
    except sqlite3.Error as exc:
        return 'sqlite fts5', False, REQUIRED, f'SQLite {sqlite3.sqlite_version}: {exc}; memory needs SQLite 3.34+ with FTS5'
    finally:
        db.close()


def home_writable():
    home = axiom_home()
    try:
        home.mkdir(parents=True, exist_ok=True)
        probe = home / '.write-probe'
        probe.write_text('ok')
        probe.unlink()
        return 'state', True, REQUIRED, str(home)
    except OSError as exc:
        return 'state', False, REQUIRED, f'{home} is not writable: {exc}'


def checks():
    found = [
        tool('claude', 'AXIOM_CLAUDE_CMD', 'claude', REQUIRED, 'runs every session', 'Install Claude Code and sign in with `claude`.'),
        tool('codex', 'AXIOM_CODEX_CMD', 'codex', REQUIRED, 'terra and sol workers', 'Install the Codex CLI and sign in.'),
        tool('agy', 'AXIOM_AGY_CMD', 'agy', OPTIONAL, 'plain-language reports', 'Install Antigravity; without it reports are shown as written.'),
        tool('bun', 'AXIOM_BUN_CMD', 'bun', REQUIRED, 'orch bookkeeping', 'Install Bun, e.g. winget install Oven-sh.Bun.'),
        sqlite_trigram(),
        home_writable(),
    ]
    if os.name == 'nt':
        found.append(windows_shell())
    return found


def main(out=sys.stdout):
    results = checks()
    for label, ok, need, detail in results:
        mark = 'ok ' if ok else ('ERR' if need == REQUIRED else 'warn')
        out.write(f'{mark:4} {label:12} {detail}\n')
    return 0 if all(ok or need == OPTIONAL for _, ok, need, _ in results) else 1
