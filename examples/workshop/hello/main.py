import json
import os
import sys

probes = []
if os.path.exists("probes.json"):
    with open("probes.json", encoding="utf-8") as f:
        probes = json.load(f)
names = ["AXIOM_WORKSHOP_MODULE", "AXIOM_WORKSHOP_DATA", "HOME"]
names.extend(p["env"] for p in probes if "env" in p)
with open("started.json", "w", encoding="utf-8") as f:
    json.dump({"pid": os.getpid(), "cwd": os.getcwd(), "env": {name: os.environ.get(name) for name in names}}, f)

if not os.path.exists("probes.json"):
    probes = [{"v": 1, "id": 1, "method": "log.write", "params": {"level": "info", "message": "hello"}}]
next_id = 1
for probe in probes:
    if "method" in probe:
        request = dict(probe)
        request.setdefault("v", 1)
        request.setdefault("id", next_id)
        next_id += 1
        print(json.dumps(request), flush=True)
        line = sys.stdin.readline()
        if not line:
            sys.exit(0)
        result = {"request": request, "reply": json.loads(line)}
    elif "env" in probe:
        result = {"env": probe["env"], "value": os.environ.get(probe["env"])}
    else:
        continue
    with open("results.jsonl", "a", encoding="utf-8") as f:
        f.write(json.dumps(result) + "\n")
with open("done", "w", encoding="utf-8"):
    pass
for _ in sys.stdin:
    pass
