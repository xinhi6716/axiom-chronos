"""A wish's goal: one JSON file per Claude session that holds the session open until the goal is done or blocked."""
import json
import re

from procs import BridgeError, data_home

SESSION = re.compile(r'[A-Za-z0-9-]{1,64}')
STATUSES = ('active', 'done', 'blocked')


def path(session):
    if not SESSION.fullmatch(session or ''):
        raise BridgeError('invalid_argument', f'not a session id: {session!r}')
    return data_home() / 'goals' / f'{session}.json'


def load(session):
    file = path(session)
    return json.loads(file.read_text(encoding='utf-8')) if file.is_file() else None


def save(session, goal):
    file = path(session)
    file.parent.mkdir(parents=True, exist_ok=True)
    file.write_text(json.dumps(goal, ensure_ascii=False, indent=1), encoding='utf-8')
    return goal


def set_status(session, status, note):
    if status not in STATUSES[1:]:
        raise BridgeError('invalid_argument', f'status must be done or blocked, not {status!r}')
    if status == 'blocked' and not (note or '').strip():
        raise BridgeError('invalid_argument', 'a blocked goal needs a note saying what only the user can provide')
    goal = load(session)
    if goal is None:
        raise BridgeError('not_found', f'no goal for session {session}')
    # The bridge reads a top-level status other than completed as a failed call.
    return {'status': 'completed', 'goal': save(session, {**goal, 'status': status, 'note': note or ''})}
