"""Claude Code hooks for a wish: open its goal, keep the session working until the goal is done or blocked,
and keep Codex calls with the chronos agents."""
import json
import os
from pathlib import Path
import re
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / 'bridge'))

import goal  # noqa: E402

WISH = re.compile(r'\s*/chronos:wish\b(.*)', re.S)


def on_prompt(event):
    session, prompt = event['session_id'], event.get('prompt') or ''
    match = WISH.match(prompt)
    current = goal.load(session)
    if match:
        current = goal.save(session, {'wish': match.group(1).strip(), 'cwd': event.get('cwd'), 'status': 'active', 'amendments': []})
        text = f'Chronos goal {session} is open for this wish. Reply in the language of the wish.'
        if os.environ.get('AXIOM_HEADLESS'):
            text += ' This run is headless: ask the user nothing.'
    elif current:
        current = goal.save(session, {**current, 'status': 'active', 'amendments': current['amendments'] + [prompt]})
        text = (f'This message amends Chronos goal {session}. Carry out any work it asks for through the chronos agents, '
                'the same way as the wish. If it only asks a question, answer it and set the goal back to done.')
    else:
        return None
    return {'hookSpecificOutput': {'hookEventName': 'UserPromptSubmit', 'additionalContext': text}}


def on_stop(event):
    current = goal.load(event['session_id'])
    if not current or current['status'] != 'active':
        return None
    return {'decision': 'block', 'reason': (
        f'Chronos goal {event["session_id"]} is still active: {current["wish"]}. Take the next step through the chronos agents. '
        'When the wish is done and verified, call the bridge goal tool with status done. If it cannot go on without something '
        'only the user has, such as a credential, a payment or permission to delete their data, call it with status blocked '
        'and say what is needed.')}


def on_tool(event):
    if event.get('agent_id'):
        return None
    return {'hookSpecificOutput': {'hookEventName': 'PreToolUse', 'permissionDecision': 'deny', 'permissionDecisionReason': (
        'codex_run is for the chronos agents. Hand building to chronos:dispatcher and Astra plans to chronos:astra.')}}


def main():
    event = json.load(sys.stdin)
    handler = {'UserPromptSubmit': on_prompt, 'Stop': on_stop, 'PreToolUse': on_tool}[event['hook_event_name']]
    reply = handler(event)
    if reply:
        sys.stdout.write(json.dumps(reply, ensure_ascii=False))


if __name__ == '__main__':
    main()
