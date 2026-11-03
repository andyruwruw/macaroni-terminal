# macaroni's zsh integration, written by macaroni into a temporary folder (safe to delete; it's
# rewritten for every new terminal). macaroni starts zsh with ZDOTDIR pointing here, so zsh reads
# this file first. It puts your own ZDOTDIR back, so everything after this (.zprofile, .zshrc,
# .zlogin) is read from where it always is, reads your .zshenv, then adds prompt marks
# (OSC 133) and the working directory (OSC 7) for the prompt jumps and long-command alerts.

if [[ -n ${MACARONI_ZDOTDIR+x} ]]; then
  if [[ -n $MACARONI_ZDOTDIR ]]; then
    export ZDOTDIR=$MACARONI_ZDOTDIR
  else
    unset ZDOTDIR
  fi
  unset MACARONI_ZDOTDIR
fi

if [[ -f ${ZDOTDIR:-$HOME}/.zshenv ]]; then
  source "${ZDOTDIR:-$HOME}/.zshenv"
fi

if [[ -o interactive ]]; then
  autoload -Uz add-zsh-hook
  # Added before your .zshrc's hooks, so `$?` is still the command's status here.
  __macaroni_precmd() {
    local code=$?
    print -rn -- $'\e]133;D;'"$code"$'\a\e]7;file://'"${HOST}${PWD}"$'\a\e]133;A\a'
  }
  __macaroni_preexec() {
    print -rn -- $'\e]133;C\a'
  }
  add-zsh-hook precmd __macaroni_precmd
  add-zsh-hook preexec __macaroni_preexec
fi
