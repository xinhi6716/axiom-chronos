"""Release payload and isolated installer checks; never configure the host machine."""
import ctypes
import hashlib
import importlib.util
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('package_release', ROOT / 'tools/package_release.py')
PACKAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGE)


class PackageReleaseTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='chronos-package-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.axiom = self.root / 'dummy-axiom.exe'
        self.chronos = self.root / 'dummy-chronos.exe'
        self.linux = self.root / 'dummy-axiom'
        self.axiom.write_bytes(b'dummy axiom 2.0.0\n')
        if os.name == 'nt':
            # A harmless stand-in that rejects the "doctor" argument with a nonzero exit.
            self.axiom.write_bytes((Path(os.environ['SystemRoot']) / 'System32/whoami.exe').read_bytes())
        self.chronos.write_bytes(b'dummy chronos 2.0.0\n')
        self.linux.write_bytes(b'#!/bin/sh\necho "dummy doctor: $*"\nexit 17\n')
        self.out = self.root / 'dist'

    def package(self):
        return PACKAGE.package_release('2.0.0', self.out, self.axiom, self.chronos, self.linux)

    def test_archive_names_payloads_and_checksums(self):
        archives = self.package()
        self.assertEqual([p.name for p in archives], [
            'axiom-chronos-2.0.0-windows-x64.zip',
            'axiom-chronos-2.0.0-linux-x64.tar.gz',
        ])
        with zipfile.ZipFile(archives[0]) as z:
            self.assertCountEqual(z.namelist(), ['axiom.exe', 'chronos.exe', 'install.ps1', 'uninstall.ps1', 'INSTALL.txt'])
            self.assertEqual(z.read('axiom.exe'), self.axiom.read_bytes())
            self.assertEqual(z.read('chronos.exe'), self.chronos.read_bytes())
            self.assertIn(b'Axiom Chronos 2.0.0', z.read('INSTALL.txt'))
            for script in ('install.ps1', 'uninstall.ps1'):
                self.assertEqual(z.read(script), (PACKAGE.INSTALLERS / script).read_bytes())
        with tarfile.open(archives[1]) as tar:
            self.assertCountEqual(tar.getnames(), ['axiom', 'install.sh', 'INSTALL.txt'])
            self.assertEqual(tar.extractfile('axiom').read(), self.linux.read_bytes())
            self.assertEqual(tar.extractfile('install.sh').read(), (PACKAGE.INSTALLERS / 'install.sh').read_bytes())
            self.assertIn(b'Axiom Chronos 2.0.0', tar.extractfile('INSTALL.txt').read())
            for name in ('axiom', 'install.sh'):
                self.assertEqual(tar.getmember(name).mode, 0o755)
        # The final release also has bare binaries; all output payloads must be covered.
        (self.out / 'bare-binary').write_bytes(b'bare binary')
        self.package()
        sums = dict(line.split('  ', 1)[::-1] for line in (self.out / 'SHA256SUMS').read_text().splitlines())
        self.assertEqual(set(sums), {p.name for p in self.out.iterdir() if p.name != 'SHA256SUMS'})
        for name, digest in sums.items():
            self.assertEqual(digest, hashlib.sha256((self.out / name).read_bytes()).hexdigest())

    def test_rejects_incomplete_or_unsafe_inputs(self):
        for kwargs in ({}, {'windows_axiom': self.axiom}, {'windows_chronos': self.chronos},
                       {'linux_axiom': self.root / 'missing'}):
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                PACKAGE.package_release('2.0.0', self.out, **kwargs)
        with self.assertRaises(ValueError):
            PACKAGE.package_release('../escape', self.out, linux_axiom=self.linux)
        self.assertFalse(self.out.exists())

    def windows_payload(self):
        archive = self.package()[0]
        extracted = self.root / 'extracted'
        with zipfile.ZipFile(archive) as z:
            z.extractall(extracted)
        return extracted

    def powershell(self, script, prefix, *switches):
        return subprocess.run([
            'powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass',
            '-File', str(script), '-Prefix', str(prefix), *switches,
        ], capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=60)

    @unittest.skipUnless(os.name == 'nt', 'Windows installer requires Windows')
    def test_windows_install_upgrade_and_uninstall(self):
        extracted = self.windows_payload()
        prefix = self.root / 'temporary prefix'
        prefix.mkdir()
        for name in ('axiom.exe', 'chronos.exe'):
            (prefix / name).write_bytes(b'old 1.0.0')
        for _ in range(2):
            result = self.powershell(extracted / 'install.ps1', prefix, '-NoPath', '-NoShortcut', '-NoPlugin')
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn('Health check (axiom doctor)', result.stdout)
            self.assertIn('axiom doctor reported problems', result.stdout)
            for name in ('axiom.exe', 'chronos.exe', 'uninstall.ps1'):
                self.assertEqual((prefix / name).read_bytes(), (extracted / name).read_bytes())
        result = self.powershell(prefix / 'uninstall.ps1', prefix, '-NoPath', '-NoShortcut')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(prefix.exists())

    @unittest.skipUnless(os.name == 'nt', 'Windows file sharing requires Windows')
    def test_windows_locked_upgrade_stops_before_replacing_either_binary(self):
        extracted = self.windows_payload()
        prefix = self.root / 'locked prefix'
        prefix.mkdir()
        for name in ('axiom.exe', 'chronos.exe'):
            (prefix / name).write_bytes(b'old 1.0.0')
        kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        kernel.CreateFileW.argtypes = [ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_uint32,
                                      ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p]
        kernel.CreateFileW.restype = ctypes.c_void_p
        kernel.CloseHandle.argtypes = [ctypes.c_void_p]
        handle = kernel.CreateFileW(str(prefix / 'chronos.exe'), 0x80000000, 1, None, 3, 0, None)
        self.assertNotEqual(handle, ctypes.c_void_p(-1).value)
        try:
            result = self.powershell(extracted / 'install.ps1', prefix, '-NoPath', '-NoShortcut', '-NoPlugin')
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('Close axiom.exe and chronos.exe', result.stdout + result.stderr)
            self.assertEqual((prefix / 'axiom.exe').read_bytes(), b'old 1.0.0')
        finally:
            kernel.CloseHandle(handle)

    @unittest.skipIf(os.name == 'nt', 'POSIX installer check')
    def test_linux_install_with_failing_doctor(self):
        archive = self.package()[1]
        extracted = self.root / 'extracted'
        extracted.mkdir()
        with tarfile.open(archive) as tar:
            for name in ('axiom', 'install.sh', 'INSTALL.txt'):
                (extracted / name).write_bytes(tar.extractfile(name).read())
        prefix = self.root / 'temporary prefix'
        for _ in range(2):
            result = subprocess.run(['sh', str(extracted / 'install.sh'), '--prefix', str(prefix), '--no-plugin'],
                                    capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=30)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn('dummy doctor: doctor', result.stdout)
            self.assertIn('doctor reported problems', result.stderr)
            self.assertEqual((prefix / 'axiom').read_bytes(), self.linux.read_bytes())
            self.assertTrue(os.access(prefix / 'axiom', os.X_OK))


if __name__ == '__main__':
    unittest.main()
