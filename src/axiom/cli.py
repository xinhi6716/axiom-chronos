"""The axiom command."""
import argparse
import os
import sys

from axiom import __version__, doctor, session


def main(argv=None):
    for stream in (sys.stdout, sys.stderr):
        # Windows consoles default to a legacy code page; wishes and answers are often not in it.
        if hasattr(stream, 'reconfigure'):
            stream.reconfigure(encoding='utf-8', errors='replace')
    parser = argparse.ArgumentParser(prog='axiom', description='Axiom Chronos: grant wishes on a codebase.')
    parser.add_argument('--version', action='version', version=f'axiom-chronos {__version__}')
    commands = parser.add_subparsers(dest='command', required=True)

    wish = commands.add_parser('wish', help='carry out a wish on the project: dispatch, verify, explain, remember')
    wish.add_argument('--project', default=os.getcwd(), help='project directory (default: current directory)')
    wish.add_argument('--model', default='opus', help='model for the core session (default: opus)')
    wish.add_argument('--max-turns', type=int, default=60)
    # Everything after the options is the wish, so a wish that names a flag such as --pending stays intact.
    wish.add_argument('text', nargs=argparse.REMAINDER, help='the wish, in any language; put options before it')

    commands.add_parser('doctor', help='check the tools and state a wish needs')

    args = parser.parse_args(argv)
    if args.command == 'doctor':
        return doctor.main()
    if not args.text:
        parser.error('wish: say what you want, e.g. axiom wish add a --stats flag to cli.py')
    project = os.path.abspath(args.project)
    if not os.path.isdir(project):
        parser.error(f'--project is not a directory: {project}')
    try:
        return session.run('wish', ' '.join(args.text), project, args.model, args.max_turns)
    except session.SessionError as exc:
        sys.stderr.write(f'axiom: {exc}\n')
        return 2
