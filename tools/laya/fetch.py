#!/usr/bin/env python3
"""Fetch pinned Laya artifacts; no network is used by Laya at runtime."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import shutil
import sys
import tarfile
import tempfile
import urllib.parse
import urllib.request
import zipfile

REPO = Path(__file__).resolve().parents[2]
PINS = Path(__file__).with_name('pins.json')
SOURCE = 'convaiinnovations/laya-multilingual'


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as f:
        for block in iter(lambda: f.read(1024 * 1024), b''):
            digest.update(block)
    return digest.hexdigest()


def verify_or_record(path, entry):
    """Verify a pin, or fill an empty pin from the downloaded file (TOFU)."""
    path = Path(path)
    digest = sha256_file(path)
    size = path.stat().st_size
    if entry.get('sha256') and (entry['sha256'] != digest or
                                (entry.get('size') is not None and entry['size'] != size)):
        raise ValueError(f'{path.name}: sha256 or size mismatch')
    entry['sha256'] = digest
    entry['size'] = size
    return digest


def write_manifest(directory, revision, layout, source, files):
    manifest = {'format': 1, 'model_version': revision, 'layout': layout,
                'source': source, 'revision': revision, 'files': files}
    (Path(directory) / 'axiom-laya.json').write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + '\n', encoding='utf-8')
    return manifest


def download(url, path):
    print(f'Downloading {url}', flush=True)
    request = urllib.request.Request(url, headers={'User-Agent': 'axiom-laya-fetch/1'})
    with urllib.request.urlopen(request, timeout=60) as response, open(path, 'wb') as out:
        shutil.copyfileobj(response, out, length=1024 * 1024)


def safe_name(name):
    path = PurePosixPath(name)
    if not name or path.is_absolute() or any(p in ('..', '.') for p in path.parts) or '\\' in name or ':' in name:
        raise ValueError(f'unsafe artifact path: {name}')
    return path


def fetch_runtime(pins, stage):
    system = platform.system()
    machine = platform.machine().lower()
    if machine not in ('x86_64', 'amd64') or system not in ('Windows', 'Linux'):
        raise ValueError(f'unsupported runtime platform: {system} {machine}')
    key = 'win-x64' if system == 'Windows' else 'linux-x64'
    entry = pins['runtime']['archives'][key]
    archive = stage / PurePosixPath(urllib.parse.urlparse(entry['url']).path).name
    download(entry['url'], archive)
    verify_or_record(archive, entry)
    target = stage / 'runtime'
    target.mkdir()
    if system == 'Windows':
        with zipfile.ZipFile(archive) as z:
            members = [n for n in z.namelist() if PurePosixPath(n).name == 'onnxruntime.dll']
            if len(members) != 1:
                raise ValueError('archive must contain one onnxruntime.dll')
            with z.open(members[0]) as src, open(target / 'onnxruntime.dll', 'wb') as dst:
                shutil.copyfileobj(src, dst)
    else:
        with tarfile.open(archive, 'r:gz') as t:
            members = [m for m in t.getmembers() if PurePosixPath(m.name).name.startswith('libonnxruntime.so') and (m.isfile() or m.issym() or m.islnk())]
            if not any(PurePosixPath(m.name).name == 'libonnxruntime.so' for m in members):
                raise ValueError('archive has no libonnxruntime.so')
            for m in members:
                name = PurePosixPath(m.name).name
                with t.extractfile(m) as src, open(target / name, 'wb') as dst:
                    shutil.copyfileobj(src, dst)
    files = pins['runtime']['files'].setdefault(key, {})
    actual = {p.name for p in target.iterdir()}
    if files and set(files) != actual:
        raise ValueError(f'{key}: runtime file list mismatch')
    for p in target.iterdir():
        file_pin = files.setdefault(p.name, {'url': entry['url'], 'revision': entry['revision'], 'sha256': None, 'size': None})
        verify_or_record(p, file_pin)
    return target


def discover_model(model):
    url = 'https://huggingface.co/api/models/' + SOURCE
    req = urllib.request.Request(url, headers={'User-Agent': 'axiom-laya-fetch/1'})
    with urllib.request.urlopen(req, timeout=60) as response:
        info = json.load(response)
    revision = info['sha']
    if len(revision) != 40 or any(c not in '0123456789abcdef' for c in revision):
        raise ValueError('HF API did not return a commit SHA')
    names = [item['rfilename'] for item in info['siblings']]
    # Preserve the ONNX layout while removing its common onnx/ prefix.
    onnx = [n for n in names if n.startswith('onnx/')]
    selected = onnx + [n for n in names if n in ('tokenizer.json', 'tokenizer_config.json', 'special_tokens_map.json', 'rl_agent_config.json') or n.startswith('tokenizer/')]
    if not onnx:
        selected += [n for n in names if n.endswith(('.onnx', '.onnx.data'))]
    files = {}
    for remote in sorted(set(selected)):
        local = remote[5:] if remote.startswith('onnx/') else remote
        safe_name(local)
        if local in files:
            continue
        files[local] = {'url': f'https://huggingface.co/{SOURCE}/resolve/{revision}/' + urllib.parse.quote(remote, safe='/'),
                        'revision': revision, 'sha256': None, 'size': None}
    basenames = {PurePosixPath(n).name for n in files}
    if 'model.onnx' in basenames:
        layout = 'fused'
    elif {'encoder.onnx', 'head.onnx'} <= basenames:
        layout = 'split'
    else:
        raise ValueError('HF repository has no recognized ONNX layout')
    if 'tokenizer.json' not in basenames or 'rl_agent_config.json' not in basenames:
        raise ValueError('HF repository is missing tokenizer.json or rl_agent_config.json')
    model.update(revision=revision, layout=layout, files=files)


def fetch_model(model, stage):
    if not model.get('revision'):
        discover_model(model)
    if not model.get('files'):
        raise ValueError('pinned model has no files')
    target = stage / 'model'
    target.mkdir()
    hashes = {}
    for name, entry in sorted(model['files'].items()):
        path = target.joinpath(*safe_name(name).parts)
        path.parent.mkdir(parents=True, exist_ok=True)
        download(entry['url'], path)
        hashes[name] = verify_or_record(path, entry)
    write_manifest(target, model['revision'], model['layout'], SOURCE, hashes)
    return target


def install(staged, destination):
    # All downloads and hashes are checked before any destination is touched.
    destination.parent.mkdir(parents=True, exist_ok=True)
    backup = staged.parent / (staged.name + '-old')
    if destination.exists():
        os.replace(destination, backup)
    try:
        os.replace(staged, destination)
    except BaseException:
        if backup.exists():
            os.replace(backup, destination)
        raise


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runtime', action='store_true')
    parser.add_argument('--model', choices=['laya-multilingual'])
    parser.add_argument('--dest', type=Path, default=REPO / '.laya')
    args = parser.parse_args(argv)
    if not args.runtime and not args.model:
        args.model = 'laya-multilingual'
    try:
        pins = json.loads(PINS.read_text(encoding='utf-8'))
        args.dest.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='.fetch-', dir=args.dest) as tmp:
            stage = Path(tmp)
            runtime = fetch_runtime(pins, stage) if args.runtime else None
            model = fetch_model(pins['model']['laya-multilingual'], stage) if args.model else None
            if runtime:
                install(runtime, args.dest / 'runtime')
            if model:
                install(model, args.dest / 'models' / args.model)
            PINS.write_text(json.dumps(pins, indent=2, sort_keys=True) + '\n', encoding='utf-8')
        return 0
    except (OSError, ValueError, KeyError, tarfile.TarError, zipfile.BadZipFile) as exc:
        print(f'fetch failed: {exc}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
