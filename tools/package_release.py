"""Build installable release archives using only the Python standard library."""
import argparse
import hashlib
import io
from pathlib import Path
import re
import tarfile
import zipfile

INSTALLERS = Path(__file__).resolve().parent / 'installers'


def package_release(version, out_dir, windows_axiom=None, windows_chronos=None,
                    linux_axiom=None):
    if not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?', version):
        raise ValueError('version must be a release number, such as 2.0.0')
    if bool(windows_axiom) != bool(windows_chronos):
        raise ValueError('Windows packaging needs both axiom.exe and chronos.exe')
    if not windows_axiom and not linux_axiom:
        raise ValueError('provide Windows and/or Linux binary paths')
    for binary in (windows_axiom, windows_chronos, linux_axiom):
        if binary and not Path(binary).is_file():
            raise ValueError(f'binary not found: {binary}')
    out_dir = Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    archives = []
    if windows_axiom:
        archive = out_dir / f'axiom-chronos-{version}-windows-x64.zip'
        with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED, compresslevel=9) as z:
            z.write(windows_axiom, 'axiom.exe')
            z.write(windows_chronos, 'chronos.exe')
            for name in ('install.ps1', 'uninstall.ps1'):
                z.write(INSTALLERS / name, name)
            z.writestr('INSTALL.txt', install_text('windows', version))
        archives.append(archive)
    if linux_axiom:
        archive = out_dir / f'axiom-chronos-{version}-linux-x64.tar.gz'
        with tarfile.open(archive, 'w:gz') as tar:
            for source, name, mode in ((Path(linux_axiom), 'axiom', 0o755),
                                       (INSTALLERS / 'install.sh', 'install.sh', 0o755)):
                info = tar.gettarinfo(str(source), arcname=name)
                info.mode = mode
                with source.open('rb') as stream:
                    tar.addfile(info, stream)
            content = install_text('linux', version).encode('utf-8')
            info = tarfile.TarInfo('INSTALL.txt')
            info.size, info.mode = len(content), 0o644
            tar.addfile(info, io.BytesIO(content))
        archives.append(archive)
    # Include all payload files in this output folder, never the checksum file itself.
    sums = []
    for path in sorted(out_dir.iterdir()):
        if path.is_file() and path.name != 'SHA256SUMS':
            # hashlib.file_digest needs Python 3.11; read in chunks so 3.8+ works.
            hasher = hashlib.sha256()
            with path.open('rb') as stream:
                for chunk in iter(lambda: stream.read(1 << 20), b''):
                    hasher.update(chunk)
            digest = hasher.hexdigest()
            sums.append(f'{digest}  {path.name}\n')
    (out_dir / 'SHA256SUMS').write_text(''.join(sums), encoding='utf-8', newline='\n')
    return archives


def install_text(platform, version):
    return (INSTALLERS / f'INSTALL-{platform}.txt').read_text(encoding='utf-8').replace('@VERSION@', version)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--version', required=True)
    parser.add_argument('--out-dir', type=Path, required=True)
    parser.add_argument('--windows-axiom', type=Path)
    parser.add_argument('--windows-chronos', type=Path)
    parser.add_argument('--linux-axiom', type=Path)
    args = parser.parse_args()
    try:
        archives = package_release(**vars(args))
    except ValueError as error:
        parser.error(str(error))
    for archive in archives:
        print(f'{archive.name}: {archive.stat().st_size} bytes')


if __name__ == '__main__':
    main()
