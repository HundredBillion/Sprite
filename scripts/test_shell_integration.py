import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
INTEGRATION = REPO / 'crates/sprite-term/shell-integration'


class ShellIntegrationTests(unittest.TestCase):
    def run_shell(self, name, code, interactive=False):
        binary = os.environ.get('SPRITE_TEST_' + name.upper()) or shutil.which(name)
        if not binary:
            self.skipTest(name + ' is unavailable; set SPRITE_TEST_' + name.upper())
        with tempfile.TemporaryDirectory(prefix='sprite shell ') as home:
            env = dict(os.environ, HOME=home, XDG_CONFIG_HOME=home + '/config',
                       XDG_DATA_HOME=home + '/data', TERM='dumb',
                       INTEGRATION=str(INTEGRATION / ('sprite.' + name)), TEST_SHELL=binary)
            env.pop('SPRITE_SHELL_INTEGRATION', None)
            if os.environ.get('SPRITE_TEST_ZSH_FPATH'):
                env['FPATH'] = os.environ['SPRITE_TEST_ZSH_FPATH']
            flags = ['--no-config'] if name == 'fish' else ['-f']
            if interactive:
                flags += ['-i']
            return subprocess.run([binary, *flags, '-c', code], env=env,
                                  capture_output=True, timeout=10)

    def test_nested_fish_installs_its_own_prompt_hooks(self):
        result = self.run_shell('fish', '''source $INTEGRATION
$TEST_SHELL --no-config -c 'source $INTEGRATION; functions -q __sprite_precmd; echo nested_hook_status:$status; emit fish_prompt'
''')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(b'nested_hook_status:0', result.stdout)
        self.assertIn(b'\x1b]133;A\x07', result.stdout)
        self.assertEqual(result.stderr, b'')

    def test_fish_inherited_marker_does_not_suppress_hooks_and_is_unexported(self):
        result = self.run_shell('fish', '''set -gx SPRITE_SHELL_INTEGRATION 1
source $INTEGRATION
functions -q __sprite_precmd; echo hook_status:$status
$TEST_SHELL --no-config -c 'set -q SPRITE_SHELL_INTEGRATION; echo inherited_status:$status'
''')
        self.assertIn(b'hook_status:0', result.stdout)
        self.assertIn(b'inherited_status:1', result.stdout)
        self.assertEqual(result.stderr, b'')

    def test_fish_repeat_source_emits_one_prompt_and_keeps_running(self):
        result = self.run_shell('fish', '''source $INTEGRATION
source $INTEGRATION
emit fish_prompt
echo survived
''', interactive=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(b'survived', result.stdout)
        self.assertEqual(result.stdout.count(b'\x1b]133;A\x07'), 1)
        self.assertEqual(result.stderr, b'')

    def test_zsh_prompt_preserves_exit_code_and_emits_completion(self):
        result = self.run_shell('zsh', '''source "$INTEGRATION"
false
__sprite_precmd
print survived
''')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, b'')
        self.assertIn(b'\x1b]133;D;1\x07', result.stdout)
        self.assertIn(b'\x1b]7;file://', result.stdout)
        self.assertIn(b'\x1b]133;A\x07', result.stdout)
        self.assertIn(b'survived', result.stdout)

    def test_zsh_repeat_source_installs_each_hook_once(self):
        result = self.run_shell('zsh', '''source "$INTEGRATION"
source "$INTEGRATION"
print -l $precmd_functions $preexec_functions
''', interactive=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines().count(b'__sprite_precmd'), 1)
        self.assertEqual(result.stdout.splitlines().count(b'__sprite_preexec'), 1)
        self.assertEqual(result.stderr, b'')
