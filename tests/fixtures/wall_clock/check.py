import pathlib
import sys

mode, *paths = sys.argv[1:]
assert mode in {"verify", "accept", "full"}
assert paths and all(pathlib.Path(path).is_file() for path in paths), (mode, paths)
print(f"{mode}: ok")
