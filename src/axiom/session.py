"""Drive one headless Claude Code session with the Axiom engine loaded, and show its progress."""
import json
import os
import subprocess
import sys
import time

from axiom.home import ENGINE, axiom_home, resolve

PLUGIN = 'ouroboros'
BRIDGE = f'mcp__plugin_{PLUGIN}_bridge__'
BRIDGE_SERVER = f'plugin:{PLUGIN}:bridge'
TOOLS = ['Agent', 'Read', 'Glob', 'Grep', 'Write', 'Bash'] + [
    BRIDGE + name for name in ('codex_run', 'gemini_preview', 'orch', 'memory_record', 'memory_search', 'memory_get', 'memory_digest')]


class SessionError(Exception):
    pass


def command(skill, argument, model, max_turns):
    claude = resolve('AXIOM_CLAUDE_CMD', 'claude')
    if claude is None:
        raise SessionError('claude is not on PATH; install Claude Code and sign in with `claude`')
    # Only the engine and local settings load: the user's own plugins, skills and hooks stay out of the session.
    return claude + [
        '-p', f'/{PLUGIN}:{skill} {argument}',
        '--plugin-dir', str(ENGINE),
        '--setting-sources', 'local',
        '--add-dir', str(axiom_home()),
        '--allowedTools', ','.join(TOOLS),
        '--output-format', 'stream-json', '--verbose',
        '--model', model,
        '--max-turns', str(max_turns),
    ]


def environment():
    home = axiom_home()
    home.mkdir(parents=True, exist_ok=True)
    return {**os.environ, 'AXIOM_DATA_HOME': str(home), 'AXIOM_PYTHON': sys.executable}


def run(skill, argument, project, model='opus', max_turns=60, out=None):
    """Returns the process exit status: 0 when the session ended without error."""
    out = out or sys.stdout
    args = command(skill, argument, model, max_turns)
    logs = axiom_home() / 'logs'
    logs.mkdir(parents=True, exist_ok=True)
    log_path = logs / f'{time.strftime("%Y%m%d-%H%M%S")}-{skill}.jsonl'
    render = Renderer(out)
    with log_path.open('w', encoding='utf-8') as log, subprocess.Popen(
            args, cwd=project, env=environment(), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            text=True, encoding='utf-8', errors='replace') as proc:
        for line in proc.stdout:
            log.write(line)
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            render.event(event)
    if render.result is None:
        out.write(f'\nclaude exited {proc.returncode} without a result; raw events are in {log_path}\n')
        return proc.returncode or 1
    out.write(f'\nlog: {log_path}\n')
    return 1 if render.result.get('is_error') else 0


class Renderer:
    """Turns stream-json events into one line per meaningful step; the final answer is printed in full."""

    def __init__(self, out):
        self.out = out
        self.result = None
        self.agents = {}

    def line(self, depth, text):
        self.out.write('  ' * depth + text + '\n')
        self.out.flush()

    def event(self, event):
        kind = event.get('type')
        if kind == 'system' and event.get('subtype') == 'init':
            bridge = next((s for s in event.get('mcp_servers') or [] if s.get('name') == BRIDGE_SERVER), None)
            if bridge is None or bridge.get('status') != 'connected':
                status = bridge.get('status') if bridge else 'missing'
                self.line(0, f'! the {BRIDGE_SERVER} MCP server is {status}; run `axiom doctor`')
        elif kind == 'assistant':
            depth = 1 if event.get('parent_tool_use_id') else 0
            for block in (event.get('message') or {}).get('content') or []:
                if block.get('type') == 'tool_use':
                    text = self.describe(block)
                    if text:
                        self.line(depth, text)
        elif kind == 'user':
            for block in (event.get('message') or {}).get('content') or []:
                if block.get('type') == 'tool_result' and block.get('tool_use_id') in self.agents:
                    self.line(0, f'< {self.agents.pop(block["tool_use_id"])} finished')
        elif kind == 'result':
            self.result = event
            self.out.write('\n' + (event.get('result') or '') + '\n')
            usage = ', '.join(f'{model} ${info.get("costUSD", 0):.2f}' for model, info in (event.get('modelUsage') or {}).items())
            self.out.write(f'\n{event.get("duration_ms", 0) / 1000:.0f}s · ${event.get("total_cost_usd") or 0:.2f}' + (f' ({usage})' if usage else '') + '\n')

    def describe(self, block):
        name, args = block.get('name', ''), block.get('input') or {}
        if name == 'Agent':
            agent = args.get('subagent_type', 'agent')
            self.agents[block.get('id')] = agent
            return f'> {agent}: {args.get("description", "")}'
        if name.startswith(BRIDGE):
            tool = name[len(BRIDGE):]
            if tool == 'codex_run':
                goal = next((l for l in (args.get('brief') or '').splitlines() if l.startswith('GOAL')), '').removeprefix('GOAL').strip()
                return f'- codex {args.get("role") or args.get("model")}: {goal[:100]}'
            if tool == 'orch':
                return f'- orch {" ".join(args.get("args") or [])[:100]}'
            return f'- {tool}'
        if name == 'Bash':
            return f'- $ {args.get("command", "")[:100]}'
        return None
