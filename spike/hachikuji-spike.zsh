# hachikuji Milestone 0 spike: a popup that ZLE draws through POSTDISPLAY (method a).
# Pure zsh, 8 fake items, no helper. Source it before zsh-syntax-highlighting.
# ponytail: fake items and the inline renderer go away in Milestone 1 (the helper renders rows).

autoload -Uz add-zle-hook-widget add-zsh-hook

# name:kind:description. Kinds: s subcommand, o option, d folder, f file.
typeset -ga _hk_all=(
  'add:s:Add file contents to the index'
  'branch:s:List, create, or delete branches'
  'checkout:s:Switch branches or restore files'
  'commit:s:Record changes to the repository'
  '--help:o:Show help'
  '--version:o:Show the version'
  'src:d:Folder'
  'README.md:f:File'
)
typeset -g _hk_key= _hk_popup=          # last BUFFER/CURSOR seen, popup text now in POSTDISPLAY
typeset -g _hk_main=emacs               # keymap that main links to; $KEYMAP only says "main"
typeset -gi _hk_open=0 _hk_sel=1 _hk_moved=0
typeset -ga _hk_items _hk_hl
typeset -gA _hk_prev _hk_role=(
  $'\t' tab  $'\r' enter  $'\e' esc
  $'\e[A' up  $'\eOA' up  $'\x10' up
  $'\e[B' down  $'\eOB' down  $'\x0e' down
)

# Build the popup text and its region_highlight entries. $1 = offset of the popup in the display.
_hk_render() {
  local -i base=$1 w iw nw=0 rows first i r
  (( w = COLUMNS - 1 < 50 ? COLUMNS - 1 : 50, iw = w - 2 ))
  (( rows = $#_hk_items < 8 ? $#_hk_items : 8 ))
  (( rows > LINES - 5 )) && (( rows = LINES - 5 ))
  (( w < 12 || rows < 1 )) && return
  (( first = _hk_sel > rows ? _hk_sel - rows + 1 : 1 ))
  local item name kind desc icon color line m=memo=hachikuji
  for item in $_hk_items; do name=${item%%:*}; (( $#name > nw )) && nw=$#name; done
  (( nw > iw / 2 )) && (( nw = iw / 2 ))
  local bar=${(l:iw::─:):-} out=$'\n'
  out+="╭${bar}╮"
  _hk_hl=( "$((base + 1)) $((base + 1 + w)) fg=8 $m" )
  for (( i = first; i < first + rows; i++ )); do
    item=$_hk_items[i] name=${item%%:*} item=${item#*:} kind=${item%%:*} desc=${item#*:}
    case $kind in
      s) icon=$'' color=fg=4 ;;
      o) icon=$'' color=fg=5 ;;
      d) icon=$'' color=fg=6 ;;
      *) icon=$'' color=fg=default ;;
    esac
    line=" $icon ${(r:nw:)name}  $desc"
    (( r = base + $#out + 1 ))  # offset of the left border of this row
    out+=$'\n'"│${(r:iw:)line}│"
    if (( i == _hk_sel )); then
      _hk_hl+=( "$r $((r + 1)) fg=8 $m" "$((r + 1)) $((r + 1 + iw)) fg=default,standout $m" "$((r + 1 + iw)) $((r + 2 + iw)) fg=8 $m" )
    else
      # Kind color on the icon only. Names and descriptions use the default foreground:
      # palette 8 text is 2.5:1 in Catppuccin Mocha and 1.9:1 in Latte (not readable).
      _hk_hl+=( "$r $((r + 1)) fg=8 $m" "$((r + 2)) $((r + 3)) $color $m" "$((r + 1 + iw)) $((r + 2 + iw)) fg=8 $m" )
    fi
  done
  (( r = base + $#out + 1 ))
  out+=$'\n'"╰${bar}╯"
  _hk_hl+=( "$r $((r + w)) fg=8 $m" )
  _hk_popup=$out
}

# Put the popup after the ghost text of zsh-autosuggestions (if any), or remove it.
_hk_draw() {
  local ghost=${POSTDISPLAY%"$_hk_popup"}
  region_highlight=( ${region_highlight:#*memo=hachikuji} )
  _hk_popup=
  if (( _hk_open )); then
    _hk_render $(( $#BUFFER + $#ghost ))
    [[ -n $_hk_popup ]] && region_highlight+=( $_hk_hl )
  fi
  POSTDISPLAY=$ghost$_hk_popup
}

_hk_pre_redraw() {
  local key="$CURSOR:$KEYMAP:$BUFFER"
  if [[ $key != "$_hk_key" ]]; then
    _hk_key=$key _hk_sel=1 _hk_moved=0 _hk_open=0
    # No popup on history recall (Up must keep walking history), in vi command mode, or for huge lines.
    if [[ -n $BUFFER && $KEYMAP != vicmd && $LASTWIDGET != *(history|search)* ]] && (( $#BUFFER <= 8192 )); then
      local tok=${LBUFFER##*[[:space:]]}
      _hk_items=( ${(M)_hk_all:#${(b)tok}*} )
      (( $#_hk_items )) && _hk_open=1
    fi
  fi
  _hk_draw
}

_hk_finish() { _hk_open=0 _hk_key=; _hk_draw }

_hk_accept() {
  local item=$_hk_items[_hk_sel] tok=${LBUFFER##*[[:space:]]}
  local name=${item%%:*} kind=${${item#*:}%%:*}
  LBUFFER=${LBUFFER%"$tok"}$name
  [[ $kind == d ]] && LBUFFER+=/ || LBUFFER+=' '
  # The ghost text was for the old buffer. pre-redraw opens the popup for the next token.
  _hk_open=0 _hk_popup= POSTDISPLAY=
  region_highlight=( ${region_highlight:#*memo=hachikuji} )
  (( $+widgets[autosuggest-fetch] )) && zle autosuggest-fetch
}

# Tab, Enter, Esc, Up, Down, ^N, ^P. With the popup closed, run the widget the key had before.
hachikuji-key() {
  if (( _hk_open )); then
    case $_hk_role[$KEYS] in
      up)    (( _hk_sel = _hk_sel > 1 ? _hk_sel - 1 : $#_hk_items, _hk_moved = 1 )); _hk_draw; return ;;
      down)  (( _hk_sel = _hk_sel < $#_hk_items ? _hk_sel + 1 : 1, _hk_moved = 1 )); _hk_draw; return ;;
      tab)   _hk_accept; return ;;
      esc)   _hk_open=0; _hk_draw; return ;;
      enter) (( _hk_moved )) && { _hk_accept; return }
             _hk_open=0; _hk_draw ;;
    esac
  fi
  local w=$_hk_prev[${KEYMAP/#%main/$_hk_main}:$KEYS]
  [[ $w == (|undefined-key) ]] && return
  zle $w
  # Up/Down walked history: keep the popup closed, so the next Up keeps walking.
  [[ $w == *(history|search)* ]] && _hk_key="$CURSOR:$KEYMAP:$BUFFER" _hk_open=0
}

# Widgets that zsh-autosuggestions uses to accept the ghost text (BUFFER+=POSTDISPLAY).
# Remove the popup first, so it never gets into the buffer.
hachikuji-pass() { _hk_open=0; _hk_draw; zle ${WIDGET##*hachikuji-pass-} }

() {
  local km k w line
  local -a pass=( forward-char end-of-line vi-forward-char vi-end-of-line vi-add-eol
    forward-word emacs-forward-word vi-forward-word vi-forward-word-end vi-forward-blank-word
    vi-forward-blank-word-end vi-find-next-char vi-find-next-char-skip )
  zle -N hachikuji-key
  for w in $pass; do zle -N hachikuji-pass-$w hachikuji-pass; done
  for km in emacs viins; do
    for k in ${(k)_hk_role}; do
      w=${${(z)"$(bindkey -M $km -- $k)"}[-1]}
      [[ $w == hachikuji-key ]] && continue  # sourced twice
      _hk_prev[$km:$k]=$w
      bindkey -M $km -- $k hachikuji-key
    done
    for line in ${(f)"$(bindkey -M $km)"}; do
      w=${line##* }
      (( $pass[(Ie)$w] )) && bindkey -M $km -- "${(Q)${line% *}}" hachikuji-pass-$w
    done
  done
  add-zle-hook-widget line-pre-redraw _hk_pre_redraw
  add-zle-hook-widget line-finish _hk_finish
}

# zsh-autosuggestions must not wrap our widgets: its wrapper hides POSTDISPLAY from them.
(( ${+ZSH_AUTOSUGGEST_IGNORE_WIDGETS} )) && ZSH_AUTOSUGGEST_IGNORE_WIDGETS+=('hachikuji-*')

# In async mode (the default), autosuggest-suggest sets POSTDISPLAY from a zle -F handler and
# calls zle -R, which does not run line-pre-redraw. Put the popup back after it.
hachikuji-suggest() { zle hachikuji-orig-suggest -- "$@"; _hk_draw; zle -R }

# Once, at the first prompt, when all plugins are loaded. Runs before autosuggest binds if we load first.
_hk_setup_once() {
  add-zsh-hook -d precmd _hk_setup_once
  # ponytail: read once, so `bindkey -v` in .zshrc counts but a later switch does not.
  _hk_main=${${(z)"$(bindkey -lL main)"}[3]}
  (( ${+ZSH_AUTOSUGGEST_IGNORE_WIDGETS} )) && (( ! $ZSH_AUTOSUGGEST_IGNORE_WIDGETS[(Ie)hachikuji-*] )) &&
    ZSH_AUTOSUGGEST_IGNORE_WIDGETS+=('hachikuji-*')
  if (( $+widgets[autosuggest-suggest] && ! $+widgets[hachikuji-orig-suggest] )); then
    zle -A autosuggest-suggest hachikuji-orig-suggest
    zle -N autosuggest-suggest hachikuji-suggest
  fi
  # Ctrl-C is SIGINT: no widget and no zle-line-finish runs, so the popup would stay in the
  # scrollback. On resize, ZLE redraws the old rows without line-pre-redraw, and they wrap.
  # Handle both from traps. Keep a TRAP function the user already has; skip if a string trap is set.
  local sig
  for sig in INT WINCH; do
    [[ -n ${(M)${(f)"$(trap)"}:#* $sig} ]] && continue
    (( $+functions[TRAP$sig] )) && functions -c TRAP$sig _hk_user_trap$sig
    # The explicit return matters: zsh aborts the line only if `return` runs in the TRAP function itself.
    eval "TRAP$sig() { _hk_on_signal $sig \"\$@\"; return \$? }"
  done
}

_hk_on_signal() {
  local sig=$1; shift
  # ZLE parameters are read-only in a trap, so do the work in a widget.
  zle && (( _hk_open )) && zle hachikuji-$sig
  (( $+functions[_hk_user_trap$sig] )) && { _hk_user_trap$sig "$@"; return }
  [[ $sig == INT ]] && return $(( 128 + $1 ))
  return 0
}
hachikuji-INT()   { _hk_open=0 _hk_key=; _hk_draw; zle -R }
hachikuji-WINCH() { _hk_draw; zle -R }
zle -N hachikuji-INT; zle -N hachikuji-WINCH
add-zsh-hook precmd _hk_setup_once
