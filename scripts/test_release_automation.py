import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]
PREPARE = REPO / "scripts" / "prepare_release.py"
CHECK = REPO / "scripts" / "check_release_version.py"


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


if __name__ == "__main__":
    unittest.main()
