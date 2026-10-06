import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]


class LocalPackagingTests(unittest.TestCase):
    def checkout(self, root):
        (root / 'packaging').mkdir()
        shutil.copy(REPO / 'packaging/PKGBUILD.local', root / 'packaging/PKGBUILD.local')
        if (REPO / 'packaging/prepare.sh').exists():
            shutil.copy(REPO / 'packaging/prepare.sh', root / 'packaging/prepare.sh')
        (root / 'Cargo.toml').write_text('[workspace.package]\nversion = "0.2.3"\n')

    def test_local_recipe_version_follows_workspace_for_makepkg_and_update(self):
        with tempfile.TemporaryDirectory(prefix='sprite package ') as directory:
            root = Path(directory)
            self.checkout(root)
            for explicit_startdir in (False, True):
                env = dict(os.environ)
                if explicit_startdir:
                    env['startdir'] = str(root / 'packaging')
                else:
                    env.pop('startdir', None)
                result = subprocess.run(
                    ['bash', '-c', '. ./PKGBUILD.local; printf "%s" "$pkgver"'],
                    cwd=root / 'packaging', env=env, capture_output=True, text=True, check=True,
                )
                self.assertEqual(result.stdout, '0.2.3')

    def test_fresh_local_recipe_prepares_terminfo_before_packaging(self):
        with tempfile.TemporaryDirectory(prefix='sprite package ') as directory:
            root = Path(directory)
            self.checkout(root)
            tools = root / 'tools'
            tools.mkdir()
            zig = tools / 'zig'
            zig.write_text("#!/bin/sh\nset -eu\nmkdir -p target\ncat > target/gen-terminfo <<'GEN'\n#!/bin/sh\nprintf 'generated from pinned source\\n'\nGEN\nchmod +x target/gen-terminfo\n")
            zig.chmod(0o755)
            env = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ['PATH'], startdir=str(root / 'packaging'))
            result = subprocess.run(['bash', '-c', '. ./PKGBUILD.local; prepare'], cwd=root / 'packaging', env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((root / 'target/ghostty.terminfo').read_text(), 'generated from pinned source\n')
            metadata = subprocess.run(['bash', '-c', '. ./PKGBUILD.local; printf "%s\\n" "${makedepends[@]}"'], cwd=root / 'packaging', env=env, capture_output=True, text=True, check=True)
            self.assertIn('zig', metadata.stdout.splitlines())


if __name__ == '__main__':
    unittest.main()
