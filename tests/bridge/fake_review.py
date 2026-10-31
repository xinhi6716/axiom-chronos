from pathlib import Path
import sys
source = (Path(__file__).parent.parent / 'fakes' / 'fake_codex_review.py').read_text(encoding='utf-8')
if '--actual-model' in sys.argv:
    actual = sys.argv[sys.argv.index('--actual-model') + 1]
    source = source.replace("models[thread_id] = params.get('model')", "models[thread_id] = " + repr(actual))
anchor = "    elif method == 'thread/start':"
assert source.count(anchor) == 1
source = source.replace(anchor, "    elif method == 'skills/list':\n        send({'id': message['id'], 'result': {'data': [{'cwd': params['cwds'][0], 'errors': [], 'skills': []}]}})\n" + anchor)
source = source.replace("'result': {'thread': {'id': thread_id}}", "'result': {'thread': {'id': thread_id}, 'model': models[thread_id], 'sandbox': {'type':'readOnly'}, 'approvalPolicy':'never'}")
exec(compile(source, 'fake_codex_review.py', 'exec'))
