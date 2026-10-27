"""Where Axiom keeps its engine and its state."""
import json
import os
import shutil
from pathlib import Path

ENGINE = Path(__file__).resolve().parent / 'engine'


def axiom_home():
    """Memory, orch stores and logs. AXIOM_HOME overrides the default ~/.axiom."""
    return Path(os.environ.get('AXIOM_HOME') or Path.home() / '.axiom')


def resolve(env_name, executable):
    """An env override is a JSON array of strings; otherwise the executable's path on PATH, or None."""
    override = os.environ.get(env_name)
    if override:
        command = json.loads(override)
        if not isinstance(command, list) or not command or not all(isinstance(part, str) for part in command):
            raise ValueError(f'{env_name} must be a non-empty JSON array of strings')
        return command
    found = shutil.which(executable)
    return [found] if found else None
