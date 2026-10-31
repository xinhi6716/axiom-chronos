"""Exercise pre-mode host callbacks using the shared app-server fake."""
from pathlib import Path

source = (Path(__file__).parents[1] / 'fakes' / 'fake_codex_session.py').read_text(encoding='utf-8')
source = source.replace("    elif method in ('thread/start','thread/resume'):", "    elif method=='skills/list': reply(msg,{'data':[{'cwd':params['cwds'][0],'errors':[],'skills':[]}]})\n    elif method in ('thread/start','thread/resume'):")
child_anchor = "    if not core:\n"
child_probe = '''    if not core and scenario == 'planning-policy':
        home = Path(option('--home'))
        output = option('--output')
        results = {}
        if 'astra' in role:
            results['inside'] = call(tid, turn, 'Write', {'file_path': str(home / 'architect-plan.md'), 'content': 'architect plan'})
            results['outside'] = call(tid, turn, 'Write', {'file_path': output + '.architect', 'content': 'NO'})
        else:
            results['output'] = call(tid, turn, 'Write', {'file_path': output, 'content': 'content artifact'})
            results['other'] = call(tid, turn, 'Write', {'file_path': output + '.other', 'content': 'NO'})
        Path(option('--record') + '.' + tid).write_text(json.dumps(results), encoding='utf-8')
        complete(tid, turn, 'probe complete')
        return
'''
core_anchor = "    if turns[tid]==1:\n"
core_probe = '''    if scenario == 'planning-policy':
        home = Path(option('--home'))
        output = option('--output')
        results = {}
        results['inside'] = call(tid, turn, 'Write', {'file_path': str(home / 'core-plan.md'), 'content': 'core plan'})
        results['outside'] = call(tid, turn, 'Write', {'file_path': output + '.core', 'content': 'NO'})
        results['dispatch'] = call(tid, turn, 'Agent', {'subagent_type': 'chronos:dispatcher', 'prompt': 'NO'})
        results['architect'] = call(tid, turn, 'Agent', {'subagent_type': 'chronos:architect', 'prompt': 'plan'})
        results['content_before'] = call(tid, turn, 'Agent', {'subagent_type': 'chronos:content', 'prompt': 'write', 'output_paths': [output]})
        results['mode'] = tool(tid, turn, 'goal', {'mode': 'answer'})
        results['content_after'] = call(tid, turn, 'Agent', {'subagent_type': 'chronos:content', 'prompt': 'write', 'output_paths': [output]})
        Path(option('--record') + '.core').write_text(json.dumps(results), encoding='utf-8')
        finish(tid, turn)
        return
'''
assert source.count(child_anchor) == 1 and source.count(core_anchor) == 1
source = source.replace(child_anchor, child_probe + child_anchor).replace(core_anchor, core_probe + core_anchor)
exec(compile(source, __file__, 'exec'))
