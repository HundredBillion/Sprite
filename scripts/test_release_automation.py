import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]
PREPARE = REPO / "scripts" / "prepare_release.py"
CHECK = REPO / "scripts" / "check_release_version.py"
RELEASE_WORKFLOW = REPO / ".github" / "workflows" / "release-pr.yml"


class ReleaseWorkflowTests(unittest.TestCase):
    def test_release_preparation_does_not_depend_on_rust_or_cargo(self):
        workflow = RELEASE_WORKFLOW.read_text()

        self.assertNotIn("rustup", workflow)
        self.assertNotIn("cargo metadata", workflow)

    def test_release_preparation_allows_generated_version_changes(self):
        workflow = RELEASE_WORKFLOW.read_text()

        self.assertIn(
            'if git diff --quiet; then\n'
            '            echo "No release version changes were generated" >&2\n'
            '            exit 1\n'
            '          fi',
            workflow,
        )


class PrepareReleaseTests(unittest.TestCase):
    def test_updates_workspace_package_recipe_and_readme(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            (root / "Cargo.toml").write_text(
                '[workspace.package]\nversion = "0.2.0"\n'
            )
            (root / "Cargo.lock").write_text(
                '[[package]]\nname = "sprite-app"\nversion = "0.2.0"\n\n'
                '[[package]]\nname = "sprite-pane"\nversion = "0.2.0"\n\n'
                '[[package]]\nname = "sprite-term"\nversion = "0.2.0"\n'
            )
            (root / "packaging").mkdir()
            (root / "packaging" / "PKGBUILD").write_text("pkgver=0.2.0\n")
            (root / "README.md").write_text(
                "sudo pacman -U sprite-0.2.0-1-x86_64.pkg.tar.zst\n"
            )

            subprocess.run(
                [sys.executable, str(PREPARE), "0.2.1", "--root", str(root)],
                check=True,
                capture_output=True,
                text=True,
            )

            self.assertIn('version = "0.2.1"', (root / "Cargo.toml").read_text())
            self.assertEqual(
                3,
                (root / "Cargo.lock").read_text().count('version = "0.2.1"'),
            )
            self.assertEqual(
                "pkgver=0.2.1\n",
                (root / "packaging" / "PKGBUILD").read_text(),
            )
            self.assertEqual(
                "sudo pacman -U sprite-0.2.1-1-x86_64.pkg.tar.zst\n",
                (root / "README.md").read_text(),
            )

    def test_rejects_tag_style_version_without_changing_files(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            (root / "Cargo.toml").write_text(
                '[workspace.package]\nversion = "0.2.0"\n'
            )
            (root / "packaging").mkdir()
            (root / "packaging" / "PKGBUILD").write_text("pkgver=0.2.0\n")
            (root / "README.md").write_text(
                "sudo pacman -U sprite-0.2.0-1-x86_64.pkg.tar.zst\n"
            )

            result = subprocess.run(
                [sys.executable, str(PREPARE), "v0.2.1", "--root", str(root)],
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(0, result.returncode)
            self.assertIn('version = "0.2.0"', (root / "Cargo.toml").read_text())
            self.assertEqual(
                "pkgver=0.2.0\n",
                (root / "packaging" / "PKGBUILD").read_text(),
            )

    def test_missing_release_reference_does_not_partially_update_files(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            original_cargo = '[workspace.package]\nversion = "0.2.0"\n'
            original_pkgbuild = "pkgver=0.2.0\n"
            (root / "Cargo.toml").write_text(original_cargo)
            (root / "packaging").mkdir()
            (root / "packaging" / "PKGBUILD").write_text(original_pkgbuild)
            (root / "README.md").write_text("no package command here\n")

            result = subprocess.run(
                [sys.executable, str(PREPARE), "0.2.1", "--root", str(root)],
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(0, result.returncode)
            self.assertEqual(original_cargo, (root / "Cargo.toml").read_text())
            self.assertEqual(
                original_pkgbuild,
                (root / "packaging" / "PKGBUILD").read_text(),
            )

    def test_ambiguous_release_reference_does_not_partially_update_files(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            original_cargo = '[workspace.package]\nversion = "0.2.0"\n'
            original_pkgbuild = "pkgver=0.2.0\n"
            original_readme = (
                "sprite-0.2.0-1-x86_64.pkg.tar.zst\n"
                "sprite-0.2.0-1-x86_64.pkg.tar.zst\n"
            )
            (root / "Cargo.toml").write_text(original_cargo)
            (root / "packaging").mkdir()
            (root / "packaging" / "PKGBUILD").write_text(original_pkgbuild)
            (root / "README.md").write_text(original_readme)

            result = subprocess.run(
                [sys.executable, str(PREPARE), "0.2.1", "--root", str(root)],
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(0, result.returncode)
            self.assertEqual(original_cargo, (root / "Cargo.toml").read_text())
            self.assertEqual(original_readme, (root / "README.md").read_text())

    def test_updates_workspace_packages_in_cargo_lock(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            (root / "Cargo.toml").write_text(
                '[workspace.package]\nversion = "0.2.0"\n'
            )
            (root / "Cargo.lock").write_text(
                '[[package]]\n'
                'name = "sprite-app"\n'
                'version = "0.2.0"\n\n'
                '[[package]]\n'
                'name = "sprite-pane"\n'
                'version = "0.2.0"\n\n'
                '[[package]]\n'
                'name = "sprite-term"\n'
                'version = "0.2.0"\n\n'
                '[[package]]\n'
                'name = "other-package"\n'
                'version = "0.2.0"\n'
            )
            (root / "packaging").mkdir()
            (root / "packaging" / "PKGBUILD").write_text("pkgver=0.2.0\n")
            (root / "README.md").write_text(
                "sudo pacman -U sprite-0.2.0-1-x86_64.pkg.tar.zst\n"
            )

            subprocess.run(
                [sys.executable, str(PREPARE), "0.2.1", "--root", str(root)],
                check=True,
                capture_output=True,
                text=True,
            )

            lockfile = (root / "Cargo.lock").read_text()
            self.assertEqual(3, lockfile.count('version = "0.2.1"'))
            self.assertIn('name = "other-package"\nversion = "0.2.0"', lockfile)


class ReleaseTagCheckTests(unittest.TestCase):
    def test_accepts_tag_matching_workspace_version(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            (root / "Cargo.toml").write_text(
                '[workspace.package]\nversion = "0.2.0"\n'
            )

            result = subprocess.run(
                [sys.executable, str(CHECK), "v0.2.0", "--root", str(root)],
                capture_output=True,
                text=True,
            )

            self.assertEqual(0, result.returncode, result.stderr)

    def test_rejects_tag_that_differs_from_workspace_version(self):
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            (root / "Cargo.toml").write_text(
                '[workspace.package]\nversion = "0.2.0"\n'
            )

            result = subprocess.run(
                [sys.executable, str(CHECK), "v0.2.1", "--root", str(root)],
                capture_output=True,
                text=True,
            )

            self.assertNotEqual(0, result.returncode)
            self.assertIn("does not match Cargo workspace version", result.stderr)

class ReleaseWriteRecoveryTests(unittest.TestCase):
    def fixture(self, root):
        import shutil
        (root / 'packaging').mkdir()
        for name in ('Cargo.toml', 'Cargo.lock', 'README.md', 'packaging/PKGBUILD'):
            shutil.copy(REPO / name, root / name)
        return {name: (root / name).read_bytes() for name in ('Cargo.toml', 'Cargo.lock', 'README.md', 'packaging/PKGBUILD')}

    def test_filesystem_failure_never_leaves_mixed_release_versions(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            original = self.fixture(root)
            lock = root / 'Cargo.lock'
            lock.chmod(0o444)
            result = subprocess.run([sys.executable, str(PREPARE), '0.2.3', '--root', str(root)], capture_output=True, text=True)
            if result.returncode:
                for name, content in original.items():
                    self.assertEqual((root / name).read_bytes(), content, name)
            else:
                import tomllib
                self.assertEqual(tomllib.loads((root / 'Cargo.toml').read_text())['workspace']['package']['version'], '0.2.3')
                packages = tomllib.loads((root / "Cargo.lock").read_text())["package"]
                versions = {package["name"]: package["version"] for package in packages if package["name"] in ("sprite-app", "sprite-pane", "sprite-term")}
                self.assertEqual(versions, {name: "0.2.3" for name in ("sprite-app", "sprite-pane", "sprite-term")})
            self.assertEqual(lock.stat().st_mode & 0o777, 0o444)

    def test_late_replacement_failure_restores_bytes_modes_and_cleans_staging(self):
        from unittest.mock import patch
        import os
        from scripts.prepare_release import prepare_release
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            original = self.fixture(root)
            modes = {name: (root / name).stat().st_mode for name in original}
            replace = os.replace
            calls = 0
            def fail_third(source, destination):
                nonlocal calls
                calls += 1
                if calls == 3:
                    raise PermissionError('injected replacement failure')
                return replace(source, destination)
            with patch('os.replace', side_effect=fail_third):
                with self.assertRaises(PermissionError):
                    prepare_release(root, '0.2.3')
            for name, content in original.items():
                self.assertEqual((root / name).read_bytes(), content, name)
                self.assertEqual((root / name).stat().st_mode, modes[name], name)
            self.assertEqual(sorted(str(p.relative_to(root)) for p in root.rglob('*') if p.is_file()), sorted(original))

    def test_staging_failure_preserves_originals_and_removes_partial_staging(self):
        from unittest.mock import patch
        from scripts.prepare_release import prepare_release
        temporary_file = tempfile.NamedTemporaryFile
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            original = self.fixture(root)
            calls = 0
            def fail_third(*args, **kwargs):
                nonlocal calls
                calls += 1
                if calls == 3:
                    raise PermissionError('injected staging failure')
                return temporary_file(*args, **kwargs)
            with patch('tempfile.NamedTemporaryFile', side_effect=fail_third):
                with self.assertRaises(PermissionError):
                    prepare_release(root, '0.2.3')
            for name, content in original.items():
                self.assertEqual((root / name).read_bytes(), content, name)
            self.assertEqual(sorted(str(p.relative_to(root)) for p in root.rglob('*') if p.is_file()), sorted(original))


if __name__ == "__main__":
    unittest.main()
