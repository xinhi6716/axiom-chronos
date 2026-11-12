#!/usr/bin/env python3
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time


def main():
    state = Path(os.environ["FAKE_COLAB_STATE"]).resolve()
    state.mkdir(parents=True, exist_ok=True)
    argv = sys.argv[1:]
    with (state / "calls.jsonl").open("a", encoding="utf-8") as calls:
        calls.write(json.dumps(argv) + "\n")
    if not argv:
        return 2
    sub = argv[0]
    if os.environ.get("FAKE_COLAB_FAIL") == sub:
        return 9
    remote = state / "remote"
    units_path = state / "units"
    if sub == "new":
        (state / "open").write_text("sess-1", encoding="utf-8")
        print("sess-1")
    elif sub == "upload":
        shutil.copytree(argv[2], remote, dirs_exist_ok=True)
    elif sub == "exec":
        units = float(units_path.read_text(encoding="utf-8")) if units_path.exists() else 100.0
        units -= float(os.environ.get("FAKE_COLAB_EXEC_UNITS", "1.5"))
        units_path.write_text(str(units), encoding="utf-8")
        deadline = time.monotonic() + float(os.environ.get("FAKE_COLAB_EXEC_SLEEP", "0"))
        while time.monotonic() < deadline:
            time.sleep(min(0.05, max(0, deadline - time.monotonic())))
        manifest = json.loads((remote / "manifest.json").read_text(encoding="utf-8"))
        return subprocess.run([sys.executable, "run.py", "--dataset", ".", "--out", "./out",
                               "--base", manifest["base"], "--revision", manifest["revision"],
                               "--epochs", "1", "--seed", "1", "--device", "cuda"], cwd=remote).returncode
    elif sub == "download":
        shutil.copytree(remote / argv[2], argv[3], dirs_exist_ok=True)
    elif sub == "usage":
        units = float(units_path.read_text(encoding="utf-8")) if units_path.exists() else 100.0
        print(json.dumps({"compute_units": units}))
    elif sub == "close":
        (state / "closed").write_text(argv[1], encoding="utf-8")
    else:
        print("unknown subcommand: " + sub, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
