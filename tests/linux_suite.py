#!/usr/bin/env python3
"""Run the default Cargo test suite in Linux, including from Windows."""

import argparse
import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys


def main():
    args = sys.argv[1:]
    if "--" in args:
        separator = args.index("--")
        runner_args, cargo_args = args[:separator], args[separator + 1 :]
    else:
        runner_args, cargo_args = args, []

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cargo-home")
    options = parser.parse_args(runner_args)
    root = Path(__file__).resolve().parent.parent

    if sys.platform == "win32":
        wsl = shutil.which("wsl.exe")
        if wsl is None:
            print("linux_suite: unavailable: WSL is not installed", file=sys.stderr)
            return 2
        cargo_home = options.cargo_home
        if cargo_home is None:
            cargo_home = os.environ.get("CARGO_HOME")
            if cargo_home:
                match = re.match(r"^([A-Za-z]):[\\/](.*)$", cargo_home)
                if match:
                    cargo_home = "/mnt/" + match.group(1).lower() + "/" + match.group(2).replace("\\", "/")
        command = [wsl, "--cd", str(root), "-e", "python3", "tests/linux_suite.py"]
        if cargo_home is not None:
            command.extend(["--cargo-home", cargo_home])
        command.extend(["--", *cargo_args])
        return subprocess.call(command)

    env = os.environ.copy()
    env["CARGO_BUILD_JOBS"] = "2"
    env["CARGO_INCREMENTAL"] = "0"
    if options.cargo_home is not None:
        env["CARGO_HOME"] = options.cargo_home
    if "CARGO_TARGET_DIR" not in env:
        repo_hash = hashlib.sha256(os.fsencode(str(root))).hexdigest()[:12]
        env["CARGO_TARGET_DIR"] = str(Path.home() / ".cache" / "axiom-chronos" / repo_hash)
    cargo = shutil.which("cargo") or str(Path.home() / ".cargo" / "bin" / "cargo")
    cargo = os.path.abspath(cargo)
    env["PATH"] = os.path.dirname(cargo) + os.pathsep + env.get("PATH", "")
    return subprocess.call(
        [cargo, "test", "--locked", "--offline", "--no-fail-fast", *cargo_args],
        cwd=root,
        env=env,
    )


if __name__ == "__main__":
    sys.exit(main())
