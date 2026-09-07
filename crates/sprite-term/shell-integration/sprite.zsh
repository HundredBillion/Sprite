# Sprite shell integration for Zsh. Version 1.
#
# Loaded by Sprite into shells it launches. Sprite never edits or appends to a
# user dotfile.

if [ -n "${SPRITE_SHELL_INTEGRATION:-}" ]; then
  return 0
fi
SPRITE_SHELL_INTEGRATION=1

__sprite_osc7() { printf '\033]7;file://%s%s\007' "${HOST:-}" "$PWD"; }

__sprite_preexec() { printf '\033]133;C\007'; }

__sprite_precmd() {
  local status=$?
  printf '\033]133;D;%s\007' "$status"
  __sprite_osc7
  printf '\033]133;A\007'
}

autoload -Uz add-zsh-hook 2>/dev/null && {
  add-zsh-hook precmd __sprite_precmd
  add-zsh-hook preexec __sprite_preexec
}
