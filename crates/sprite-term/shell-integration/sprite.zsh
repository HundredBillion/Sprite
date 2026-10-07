# Sprite shell integration for Zsh. Version 1.
#
# Loaded by Sprite into shells it launches. Sprite never edits or appends to a
# user dotfile.

SPRITE_SHELL_INTEGRATION=1
typeset +x SPRITE_SHELL_INTEGRATION

# An inherited marker does not mean this shell has its own prompt hooks.
if typeset -f __sprite_osc7 __sprite_preexec __sprite_precmd >/dev/null; then
  return 0
fi

__sprite_osc7() { printf '\033]7;file://%s%s\007' "${HOST:-}" "$PWD"; }

__sprite_preexec() { printf '\033]133;C\007'; }

__sprite_precmd() {
  local exit_code=$?
  printf '\033]133;D;%s\007' "$exit_code"
  __sprite_osc7
  printf '\033]133;A\007'
}

autoload -Uz add-zsh-hook 2>/dev/null && {
  add-zsh-hook precmd __sprite_precmd
  add-zsh-hook preexec __sprite_preexec
}
