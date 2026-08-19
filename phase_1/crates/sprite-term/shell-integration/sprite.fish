# Sprite shell integration for Fish. Version 1.
#
# Loaded by Sprite into shells it launches. Sprite never edits or appends to a
# user configuration file.

if set -q SPRITE_SHELL_INTEGRATION
    exit 0
end
set -gx SPRITE_SHELL_INTEGRATION 1

function __sprite_osc7 --on-variable PWD
    printf '\033]7;file://%s%s\007' (hostname) "$PWD"
end

function __sprite_preexec --on-event fish_preexec
    printf '\033]133;C\007'
end

function __sprite_precmd --on-event fish_prompt
    printf '\033]133;D;%s\007' $status
    __sprite_osc7
    printf '\033]133;A\007'
end
