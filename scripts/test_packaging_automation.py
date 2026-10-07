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


class MacosInstallTests(unittest.TestCase):
    def install(self, failure='', existing=True):
        directory = tempfile.TemporaryDirectory(prefix="sprite install ; '$ ")
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        source = root / 'new.app'
        destination = root / 'Sprite.app'
        for app, content in ((source, 'new'), (destination, 'old')):
            (app / 'Contents/MacOS').mkdir(parents=True)
            (app / 'Contents/MacOS/sprite').write_text(content)
        link = root / 'sprite'
        link.symlink_to(destination / 'Contents/MacOS/sprite')
        if not existing:
            shutil.rmtree(destination)
        tools = root / 'tools'
        tools.mkdir()
        commands = {
            'ditto': '#!/bin/sh\nif [ "$FAILURE" = copy ]; then mkdir -p "$2"; exit 73; fi\nexec cp -R "$1" "$2"\n',
            'mv': '#!/bin/sh\ncase "$1" in */Sprite.app) [ "$FAILURE" != backup ] || exit 76;; */staged.app) [ "$FAILURE" != replace ] && [ "$FAILURE" != rollback ] || exit 74;; */previous.app) [ "$FAILURE" != rollback ] || exit 75;; esac\nexec /bin/mv "$@"\n',
        }
        for name, body in commands.items():
            tool = tools / name
            tool.write_text(body)
            tool.chmod(0o755)
        result = subprocess.run(
            ['/bin/sh', str(REPO / 'packaging/macos/install.sh'), str(source), str(destination)],
            env=dict(os.environ, PATH=str(tools) + os.pathsep + os.environ['PATH'], FAILURE=failure),
            capture_output=True, text=True,
        )
        return root, link, result

    def test_failed_staged_copy_keeps_working_bundle_and_cli(self):
        root, link, result = self.install('copy')
        self.assertEqual(result.returncode, 73, result.stderr)
        self.assertEqual(link.read_text(), 'old')
        self.assertEqual(sorted(p.name for p in root.iterdir()), ['Sprite.app', 'new.app', 'sprite', 'tools'])

    def test_failed_replacement_restores_working_bundle_and_cli(self):
        root, link, result = self.install('replace')
        self.assertEqual(result.returncode, 74, result.stderr)
        self.assertEqual(link.read_text(), 'old')
        self.assertFalse(list(root.glob('.sprite-install.*')))

    def test_failed_rollback_retains_and_reports_recoverable_old_bundle(self):
        root, link, result = self.install('rollback')
        self.assertNotEqual(result.returncode, 0)
        backups = list(root.glob('.sprite-install.*/previous.app/Contents/MacOS/sprite'))
        self.assertEqual(len(backups), 1, result.stderr)
        self.assertEqual(backups[0].read_text(), 'old')
        self.assertIn(str(backups[0].parents[2]), result.stderr)

    def test_success_replaces_bundle_and_keeps_one_cli_link(self):
        root, link, result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(link.is_symlink())
        self.assertEqual(link.read_text(), 'new')
        self.assertFalse(list(root.glob('.sprite-install.*')))

    def test_failed_backup_rename_keeps_working_bundle(self):
        root, link, result = self.install('backup')
        self.assertEqual(result.returncode, 76, result.stderr)
        self.assertEqual(link.read_text(), 'old')
        self.assertFalse(list(root.glob('.sprite-install.*')))

    def test_first_install_succeeds_without_previous_bundle(self):
        root, link, result = self.install(existing=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(link.read_text(), 'new')
        self.assertFalse(list(root.glob('.sprite-install.*')))


if __name__ == '__main__':
    unittest.main()
