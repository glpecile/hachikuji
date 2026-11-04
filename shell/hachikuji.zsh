# hachikuji: IDE-style completion popup for zsh. https://github.com/glpecile/hachikuji
# Add `eval "$(hachikuji init zsh)"` to ~/.zshrc, before zsh-syntax-highlighting.
#
# The popup is drawn by ZLE: it goes in POSTDISPLAY (after the ghost text of zsh-autosuggestions)
# and is colored with region_highlight. The helper (one per shell, started on the first key)
# renders the rows; this file only places them. Every failure is silent: no helper, no popup.

[[ -o interactive ]] || return 0
zmodload zsh/datetime 2>/dev/null || return 0
autoload -Uz add-zle-hook-widget add-zsh-hook

typeset -g _hk_bin=${HACHIKUJI_BIN:-$commands[hachikuji]} _hk_src=${${(%):-%x}:A}
typeset -g _hk_key= _hk_popup= _hk_text= _hk_insert= _hk_rbuf= _hk_main=emacs
typeset -gi _hk_open=0 _hk_sel=0 _hk_moved=0 _hk_count=0 _hk_replace=0 _hk_dismissed=0
# open = "tab" in config.toml: the popup opens on Tab only, then follows typing until it closes.
typeset -gi _hk_tab=0 _hk_armed=0
typeset -g _hk_tab1=  # where a Tab did nothing: a second Tab there runs zsh completion
typeset -gi _hk_seq=0 _hk_inflight=0 _hk_pending=0 _hk_wfd=-1 _hk_rfd=-1 _hk_pid=0 _hk_started=-100
typeset -gF _hk_sent=0
typeset -ga _hk_hl
# Keys in the notation of `bindkey` and ${(V)KEYS}.
typeset -gA _hk_prev _hk_role=(
  '^I' tab  '^M' enter  '^[' esc
  '^[[A' up  '^[OA' up  '^P' up  '^[[Z' up
  '^[[B' down  '^[OB' down  '^N' down
)

# --- transport -----------------------------------------------------------------------------

_hk_start() {
  emulate -L zsh
  [[ -x $_hk_bin ]] && zmodload zsh/system zsh/zselect 2>/dev/null || return 1
  (( EPOCHSECONDS - _hk_started >= 60 )) || return 1  # restart at most once a minute
  _hk_started=$EPOCHSECONDS
  local dir k
  dir=$(command mktemp -d "${TMPDIR:-/tmp}/hachikuji.XXXXXX") 2>/dev/null || return 1
  command mkfifo -m 600 $dir/q $dir/r 2>/dev/null || { command rm -rf -- $dir; return 1 }
  # Read-write opens never block. The helper deletes the folder once it has opened both FIFOs.
  # Close-on-exec: a child process (`sleep 1000 &!`) must not keep the FIFOs, or the helper
  # outlives the shell. Non-blocking writes: a FIFO holds 8 KB, and a helper that does not read
  # must never freeze the shell (_hk_send).
  sysopen -r -w -o nonblock,cloexec -u _hk_wfd $dir/q 2>/dev/null &&
    sysopen -r -w -o cloexec -u _hk_rfd $dir/r 2>/dev/null || { _hk_stop; command rm -rf -- $dir; return 1 }
  # &! disowns it: no job entry, own process group. The helper then calls setsid().
  $_hk_bin serve $dir </dev/null >/dev/null 2>&1 {_hk_wfd}>&- {_hk_rfd}>&- &!
  _hk_pid=$!
  zle -F -w $_hk_rfd hachikuji-recv
  local -a al
  for k in ${(k)aliases}; do al+=( "alias ${(q)k}=${(q)aliases[$k]}" ); done
  _hk_send $'1\0a\0'"${(F)al[1,500]}"$'\0'
}

# Writes one frame. The helper reads all the time, so a frame larger than the FIFO goes in
# parts. A helper that reads nothing for 100 ms (stopped, gone) is stopped: the shell never waits.
_hk_send() {
  emulate -L zsh
  local LC_ALL=C data=$1  # byte offsets
  local -i n tries=0
  while (( $#data )); do
    if syswrite -c n -o $_hk_wfd -- $data 2>/dev/null; then
      data=${data[n+1,-1]}
    elif (( ++tries > 10 )); then
      _hk_stop
      return 1
    else
      zselect -t 1 -w $_hk_wfd 2>/dev/null  # up to 10 ms until the helper reads
    fi
  done
}

_hk_stop() {
  emulate -L zsh
  (( _hk_rfd >= 0 )) && { zle -F $_hk_rfd 2>/dev/null; exec {_hk_rfd}<&- }
  (( _hk_wfd >= 0 )) && exec {_hk_wfd}>&-
  (( _hk_pid > 0 )) && kill -9 $_hk_pid 2>/dev/null
  _hk_rfd=-1 _hk_wfd=-1 _hk_pid=0 _hk_inflight=0 _hk_pending=0 _hk_open=0 _hk_rbuf=
}

# One request in flight at most. While one is in flight, only remember that a newer one is due.
_hk_request() {
  emulate -L zsh
  if (( _hk_wfd < 0 )); then _hk_start || return; fi
  if (( _hk_inflight )); then _hk_pending=1; return; fi
  local -i bytes
  () { local LC_ALL=C; bytes=${#1} } "$LBUFFER"  # CURSOR counts characters, the helper wants bytes
  _hk_pending=0 _hk_inflight=$(( ++_hk_seq )) _hk_sent=$EPOCHREALTIME
  _hk_send "1"$'\0'q$'\0'$_hk_seq$'\0'$bytes$'\0'$PWD$'\0'$COLUMNS$'\0'$LINES$'\0'$_hk_sel$'\0'$BUFFER$'\0'
}

# Reads what the helper wrote and handles each complete frame. $1: seconds to wait for data.
_hk_read() {
  emulate -L zsh
  (( _hk_rfd >= 0 )) || return
  local chunk
  sysread -s 65536 -t ${1:-0} -i $_hk_rfd chunk
  case $? in
    0) _hk_rbuf+=$chunk ;;
    4) return ;;            # no data yet
    *) _hk_stop; return ;;  # EOF or error: the helper is gone
  esac
  # Split once: pattern removal (${buf%%$'\x1e'*}) took 13 ms on a 1 KB response.
  local -a parts=( "${(@ps:\x1e:)_hk_rbuf}" )
  _hk_rbuf=$parts[-1]  # the part after the last frame end: empty or incomplete
  local frame
  for frame in "${(@)parts[1,-2]}"; do _hk_frame "$frame"; done
}

_hk_frame() {
  emulate -L zsh
  local -a f=( "${(@ps:\x1f:)1}" )
  [[ $f[1] == 1 ]] || { _hk_stop; return }  # another protocol version
  [[ $f[3] == h ]] && { _hk_pid=$f[4]; return }
  (( f[2] == _hk_inflight )) && _hk_inflight=0
  # The buffer changed since: ask again and drop this one.
  (( _hk_pending && ! _hk_inflight )) && { _hk_request; return }
  (( f[2] != _hk_seq || _hk_dismissed )) && return  # an older request, or closed with Esc
  _hk_count=$f[4] _hk_sel=$f[5] _hk_replace=$f[6] _hk_insert=$f[7] _hk_text=$f[8]
  _hk_hl=( ${(f)f[9]} )
  _hk_open=$(( _hk_count > 0 ))
}

hachikuji-recv() { emulate -L zsh; _hk_read 0; _hk_draw; zle -R }

# Waits up to 100 ms for the newest response, so Tab and Enter act on what the user sees.
_hk_sync() {
  emulate -L zsh
  local -i n=0
  while (( _hk_inflight && n++ < 10 )); do _hk_read 0.01; done
}

# --- drawing -------------------------------------------------------------------------------

# Puts the popup after the ghost text of zsh-autosuggestions (if any), or removes it.
_hk_draw() {
  emulate -L zsh
  local ghost=${POSTDISPLAY%"$_hk_popup"} e
  region_highlight=( ${region_highlight:#*memo=hachikuji} )
  _hk_popup=
  if (( _hk_open )) && [[ -n $_hk_text ]]; then
    local -i base=$(( $#BUFFER + $#ghost ))
    local -a s
    for e in $_hk_hl; do
      s=( ${=e} )
      region_highlight+=( "$(( base + s[1] )) $(( base + s[2] )) $s[3] memo=hachikuji" )
    done
    _hk_popup=$_hk_text
  fi
  POSTDISPLAY=$ghost$_hk_popup
}

_hk_pre_redraw() {
  emulate -L zsh
  # No response in 1 s: the helper hangs. Kill it; no popup.
  (( _hk_inflight && EPOCHREALTIME - _hk_sent > 1 )) && _hk_stop
  local key="$CURSOR:$KEYMAP:$BUFFER"
  if [[ $key != "$_hk_key" ]]; then
    _hk_key=$key _hk_sel=0 _hk_moved=0 _hk_dismissed=0
    # No popup on history recall (Up must keep walking history), in vi command mode, or for huge lines.
    if [[ -n $BUFFER && $KEYMAP != vicmd && $LASTWIDGET != *(history|search)* ]] && (( $#BUFFER <= 8192 )) &&
      (( ! _hk_tab || _hk_armed )); then
      _hk_request
    else
      _hk_open=0
      [[ -z $BUFFER ]] && _hk_armed=0
    fi
  fi
  _hk_draw
}

_hk_finish() { emulate -L zsh; _hk_open=0 _hk_armed=0 _hk_key=; _hk_draw }

_hk_accept() {
  emulate -L zsh
  _hk_sync
  (( _hk_open )) || return 1
  # \x01 marks where the cursor goes: `-m '\x01'` puts it between the quotes.
  local ins=$_hk_insert post=
  [[ $ins == *$'\x01'* ]] && post=${ins#*$'\x01'} ins=${ins%%$'\x01'*}
  LBUFFER=${LBUFFER[1,-$(( _hk_replace + 1 ))]}$ins
  RBUFFER=$post$RBUFFER
  # The ghost text was for the old buffer. pre-redraw asks for the next token at once.
  _hk_open=0 _hk_popup= _hk_text= POSTDISPLAY=
  region_highlight=( ${region_highlight:#*memo=hachikuji} )
  (( $+widgets[autosuggest-fetch] )) && zle autosuggest-fetch
}

# --- keys ----------------------------------------------------------------------------------

# Tab, Enter, Esc, Up, Down, ^N, ^P, Shift-Tab. With the popup closed, run the widget the key had before.
# That widget runs with the user's options; our part runs with zsh's (sh_word_split, ksh_arrays, ...).
hachikuji-key() {
  local REPLY=
  _hk_key_press || return 0
  zle $REPLY
  _hk_key_after $REPLY
}

# Handles the key. Returns 1 when done, or 0 with the widget to run in REPLY.
_hk_key_press() {
  emulate -L zsh
  local k=${${(V)KEYS}/#%\\t/^I}  # (V) shows Tab as \t, bindkey as ^I
  # open = "tab": the first Tab opens the popup. A Tab with nothing to show runs zsh completion.
  if (( _hk_tab && ! _hk_armed && ! _hk_open )) && [[ $_hk_role[$k] == tab && -n $BUFFER ]]; then
    _hk_armed=1 _hk_key=
    return 1
  fi
  # A fast Tab after an insert: the popup for the next word is still on its way. Wait for it,
  # or Tab falls through to zsh completion ("do you wish to see all 1980 possibilities?").
  [[ $_hk_role[$k] == tab ]] && (( _hk_inflight && ! _hk_open )) && _hk_sync
  if (( _hk_open && _hk_count )); then
    case $_hk_role[$k] in
      up)    (( _hk_sel = (_hk_sel + _hk_count - 1) % _hk_count, _hk_moved = 1 )); _hk_request; return 1 ;;
      down)  (( _hk_sel = (_hk_sel + 1) % _hk_count, _hk_moved = 1 )); _hk_request; return 1 ;;
      tab)   _hk_accept && return 1 ;;
      esc)   _hk_open=0 _hk_dismissed=1 _hk_armed=0; _hk_draw; return 1 ;;
      enter) (( _hk_moved )) && _hk_accept && return 1
             _hk_open=0 _hk_dismissed=1; _hk_draw ;;
    esac
  fi
  # Popup closed: the first Tab does nothing, a second Tab runs zsh completion. A stray Tab never
  # starts "do you wish to see all 1980 possibilities?". Without a helper, Tab works as before.
  if [[ $_hk_role[$k] == tab && $KEYMAP != vicmd ]] && (( _hk_wfd >= 0 )); then
    if [[ $LASTWIDGET != hachikuji-key || $_hk_tab1 != "$CURSOR:$BUFFER" ]]; then
      _hk_tab1="$CURSOR:$BUFFER"
      return 1
    fi
    _hk_tab1=
  fi
  REPLY=$_hk_prev[${KEYMAP/#%main/$_hk_main}:$k]
  [[ $REPLY != (|undefined-key) ]]
}

# Up/Down walked history: keep the popup closed, so the next Up keeps walking.
_hk_key_after() {
  emulate -L zsh
  [[ $1 == *(history|search)* ]] && _hk_key="$CURSOR:$KEYMAP:$BUFFER" _hk_open=0
  return 0
}

# Widgets that zsh-autosuggestions uses to accept the ghost text (BUFFER+=POSTDISPLAY), and its
# own accept and execute widgets. Remove the popup first, so it never gets into the buffer.
hachikuji-pass() { _hk_open=0; _hk_draw; zle ${WIDGET##*hachikuji-pass-} }

# In async mode (the default), autosuggest-suggest sets POSTDISPLAY from a zle -F handler and
# calls zle -R, which does not run line-pre-redraw. Put the popup back after it.
hachikuji-suggest() { zle hachikuji-orig-suggest -- "$@"; emulate -L zsh; _hk_draw; zle -R }

hachikuji-INT()   { emulate -L zsh; _hk_open=0 _hk_armed=0 _hk_key=; _hk_draw; zle -R }
hachikuji-WINCH() { emulate -L zsh; _hk_request; _hk_draw; zle -R }

_hk_on_signal() {
  local sig=$1; shift
  # ZLE parameters are read-only in a trap, so do the work in a widget.
  zle && (( _hk_open )) && zle hachikuji-$sig
  (( $+functions[_hk_user_trap$sig] )) && { _hk_user_trap$sig "$@"; return }
  [[ $sig == INT ]] && return $(( 128 + $1 ))
  return 0
}

# Once, at the first prompt, when .zshrc and all plugins are loaded.
_hk_setup() {
  # local_traps (part of emulate -L) would remove the TRAPINT and TRAPWINCH set here on return.
  emulate -L zsh
  setopt no_local_traps
  add-zsh-hook -d precmd _hk_setup
  zmodload -F zsh/files b:zf_rm 2>/dev/null
  # Compile this file once: the next shells load the .zwc in 0.1 ms instead of 2 ms.
  [[ -w ${_hk_src:h} && ( ! -f $_hk_src.zwc || $_hk_src -nt $_hk_src.zwc ) ]] && zcompile $_hk_src 2>/dev/null
  local f=${TMPDIR:-/tmp}/hachikuji-keys.$$ km k w line
  local -a pass=( forward-char end-of-line vi-forward-char vi-end-of-line vi-add-eol
    forward-word emacs-forward-word vi-forward-word vi-forward-word-end vi-forward-blank-word
    vi-forward-blank-word-end vi-find-next-char vi-find-next-char-skip
    autosuggest-accept autosuggest-execute autosuggest-partial-accept
    $ZSH_AUTOSUGGEST_ACCEPT_WIDGETS $ZSH_AUTOSUGGEST_EXECUTE_WIDGETS $ZSH_AUTOSUGGEST_PARTIAL_ACCEPT_WIDGETS )
  pass=( ${(u)pass} )
  for w in $pass; do zle -N hachikuji-pass-$w hachikuji-pass; done
  # `bindkey > file` and $(<file) run without a fork, unlike $(bindkey).
  for km in emacs viins; do
    bindkey -M $km >| $f 2>/dev/null || return
    for line in ${(f)"$(<$f)"}; do
      k=${(Q)${line% *}} w=${line##* }
      if (( $+_hk_role[$k] )); then
        [[ $w == hachikuji-key ]] || _hk_prev[$km:$k]=$w
        bindkey -M $km $k hachikuji-key
      elif (( $pass[(Ie)$w] )); then
        bindkey -M $km $k hachikuji-pass-$w
      fi
    done
    # Keys with no binding (Esc in emacs) are not listed. Bind them too.
    for k in ${(k)_hk_role}; do
      (( $+_hk_prev[$km:$k] )) || { _hk_prev[$km:$k]=; bindkey -M $km $k hachikuji-key }
    done
  done
  bindkey -lL main >| $f
  _hk_main=${${(z)"$(<$f)"}[3]}
  # The `open` key of config.toml (the helper reads the other keys).
  local cfg=${XDG_CONFIG_HOME:-$HOME/.config}/hachikuji/config.toml
  [[ -r $cfg ]] && for line in ${(f)"$(<$cfg)"}; do
    [[ ${line//[[:space:]]/} == open=\"tab\"* ]] && _hk_tab=1
  done
  trap >| $f
  local traps="$(<$f)"
  zf_rm -f $f 2>/dev/null || command rm -f $f
  # zsh-autosuggestions must not wrap our widgets: its wrapper hides POSTDISPLAY from them.
  (( ${+ZSH_AUTOSUGGEST_IGNORE_WIDGETS} )) && (( ! $ZSH_AUTOSUGGEST_IGNORE_WIDGETS[(Ie)hachikuji-*] )) &&
    ZSH_AUTOSUGGEST_IGNORE_WIDGETS+=('hachikuji-*')
  if (( $+widgets[autosuggest-suggest] && ! $+widgets[hachikuji-orig-suggest] )); then
    zle -A autosuggest-suggest hachikuji-orig-suggest
    zle -N autosuggest-suggest hachikuji-suggest
  fi
  # zsh-autosuggestions highlights all of POSTDISPLAY (fg=8), and the popup is in it: the popup
  # text would turn gray. Let it see only its ghost text. (fg=default cannot override it: zsh
  # stores that as "none".)
  if (( $+functions[_zsh_autosuggest_highlight_apply] && ! $+functions[_hk_orig_highlight_apply] )); then
    functions -c _zsh_autosuggest_highlight_apply _hk_orig_highlight_apply
    _zsh_autosuggest_highlight_apply() {
      local pd=$POSTDISPLAY
      POSTDISPLAY=${POSTDISPLAY%"$_hk_popup"}
      _hk_orig_highlight_apply "$@"
      POSTDISPLAY=$pd
    }
  fi
  # Ctrl-C is SIGINT: no widget and no zle-line-finish runs, so the popup would stay in the
  # scrollback. On resize, ZLE redraws the old rows without line-pre-redraw, and they wrap.
  # Keep a TRAP function the user already has; skip if a string trap is set.
  local sig
  for sig in INT WINCH; do
    [[ -n ${(M)${(f)traps}:#* $sig} ]] && continue
    (( $+functions[TRAP$sig] )) && functions -c TRAP$sig _hk_user_trap$sig
    # The explicit return matters: zsh aborts the line only if `return` runs in the TRAP function itself.
    eval "TRAP$sig() { _hk_on_signal $sig \"\$@\"; return \$? }"
  done
}

() {
  emulate -L zsh
  zle -N hachikuji-key; zle -N hachikuji-pass; zle -N hachikuji-recv; zle -N hachikuji-suggest
  zle -N hachikuji-INT; zle -N hachikuji-WINCH
  (( ${+ZSH_AUTOSUGGEST_IGNORE_WIDGETS} )) && ZSH_AUTOSUGGEST_IGNORE_WIDGETS+=('hachikuji-*')
  add-zle-hook-widget line-pre-redraw _hk_pre_redraw
  add-zle-hook-widget line-finish _hk_finish
  add-zsh-hook precmd _hk_setup
}
