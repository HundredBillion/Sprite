# Sprite shell integration for Bash. Version 1.
#
# Loaded by Sprite into shells it launches. Sprite never edits or appends to a
# user dotfile; if this file is not loaded, the terminal simply reports less.
#
# Emits OSC 133 semantic prompt marks and OSC 7 working directory, which is what
# lets Sprite distinguish a prompt from its output without parsing text.

SPRITE_SHELL_INTEGRATION=1
export -n SPRITE_SHELL_INTEGRATION

# An inherited marker does not mean this shell has its own prompt hooks.
if declare -F __sprite_osc7 __sprite_preexec __sprite_precmd >/dev/null; then
  return 0
fi

__sprite_osc7() {
  printf '\033]7;file://%s%s\007' "${HOSTNAME:-}" "$PWD"
}

__sprite_prompt_start() { printf '\033]133;A\007'; }
__sprite_command_start() { printf '\033]133;B\007'; }
__sprite_command_output() { printf '\033]133;C\007'; }
__sprite_command_done() { printf '\033]133;D;%s\007' "$1"; }

__sprite_preexec() {
  __sprite_command_output
}

__sprite_precmd() {
  local status=$?
  __sprite_command_done "$status"
  __sprite_osc7
  __sprite_prompt_start
  return $status
}

# PROMPT_COMMAND runs before each prompt; PS0 is emitted after a command is
# entered but before it runs.
PROMPT_COMMAND="__sprite_precmd${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
PS0="\[$(__sprite_preexec)\]${PS0:-}"
PS1="\[$(__sprite_command_start)\]${PS1:-}"
