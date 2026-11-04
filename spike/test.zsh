#!/usr/bin/env zsh
# Milestone 0 check: drives a real zsh in tmux with the spike, zsh-autosuggestions (async),
# a two-line starship prompt and zsh-syntax-highlighting, then reads the screen.
# Usage: zsh spike/test.zsh   (needs tmux and starship; clones zsh-autosuggestions once)
emulate -L zsh
setopt pipe_fail extended_glob
zmodload zsh/datetime zsh/mathfunc

root=${0:A:h}
work=${TMPDIR:-/tmp}/hachikuji-m0
mkdir -p $work
[[ -d $work/zsh-autosuggestions ]] ||
  git clone -q --depth 1 https://github.com/zsh-users/zsh-autosuggestions $work/zsh-autosuggestions
zsh_hl=${HK_ZSH_HL:-$(brew --prefix)/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh}

print -r -- 'add_newline = false
format = "$directory\n$character"' > $work/starship.toml

T() { tmux -L hk-m0 -f /dev/null "$@" }
fails=0
ok()   { print -r -- "ok   $1" }
fail() { print -r -- "FAIL $1"; (( ++fails )); print -r -- "$(T capture-pane -p)" | sed 's/^/     | /' }
check() { (( $2 == 0 )) && ok $1 || fail $1 }  # check name $?
scr() { T capture-pane -p }
esc() { T capture-pane -p -e }
has() { [[ "$(scr)" == *$~1* ]] }
# Wait up to 3 s for a screen pattern.
waitfor() { local i; for i in {1..60}; do has "$1" && return 0; sleep 0.05; done; return 1 }

# start [cols] [rows] [extra zshrc lines]
start() {
  T kill-server 2>/dev/null || true
  rm -f $work/hist
  print -r -- "export LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8
HISTFILE=$work/hist SAVEHIST=100 HISTSIZE=100
print -s 'git status'; print -s 'echo hello world'; print -s 'ls --h'
$3
source $root/hachikuji-spike.zsh
source $work/zsh-autosuggestions/zsh-autosuggestions.zsh
export STARSHIP_CONFIG=$work/starship.toml
eval \"\$(starship init zsh)\"
source $zsh_hl" > $work/.zshrc
  T new-session -d -x ${1:-80} -y ${2:-24} -e ZDOTDIR=$work -c $root zsh -i
  waitfor '❯'
}

top='╭──'
bot='╰──'

# 1. popup opens with the ghost text before it, on a two-line prompt
start
T send-keys 'git '
waitfor $top
has "*❯ git status*${top}*add*README.md*${bot}*"; check 'popup opens, ghost text kept' $?
[[ "$(esc)" == *$'\e[32mgit'* ]]; check 'zsh-syntax-highlighting still colors the command' $?
[[ "$(esc | grep -aF $'\e[7m')" == *add* ]]; check 'first row selected (reverse)' $?

# 2. Down moves the selection, Tab accepts and reopens the popup for the next token
T send-keys Down
sleep 0.2
[[ "$(esc | grep -aF $'\e[7m')" == *branch* ]]; check 'Down moves selection' $?
T send-keys Tab
# capture-pane trims trailing spaces: a popup with all 8 rows proves the space (empty token).
waitfor "❯ git branch*$top*README.md"
has "❯ git branch*$top*add*README.md"; check 'Tab inserts item + space, popup reopens' $?

# 3. Enter without moving runs the line, and leaves no popup on the screen
start
T send-keys 'echo a'
waitfor $top
T send-keys Enter
waitfor $'\na\n'
has $'❯ echo a\na\n'; check 'Enter runs line when not moved' $?
! has $top; check 'no popup left after Enter' $?

# 4. Enter after a move accepts the item
start
T send-keys 'echo a'
waitfor $top
T send-keys C-n Enter
waitfor "❯ echo add*$top*README.md"
has "❯ echo add*$top*add*README.md"; check 'Enter accepts after a move' $?

# 5. Esc closes the popup (KEYTIMEOUT delay)
start
T send-keys 'git '
waitfor $top
T send-keys Escape
sleep 0.6
! has $top; check 'Esc closes popup' $?

# 6. Right arrow accepts the ghost text, and the popup never gets into the buffer
start
T send-keys 'echo '
waitfor "❯ echo hello world*$top"
T send-keys Right Enter
waitfor $'\nhello world\n'
# If the popup got into the buffer, its rows run as commands after "hello world".
has $'❯ echo hello world\nhello world\n*❯' && ! has 'not found' && ! has $top
check 'Right accepts ghost only' $?

# 7. Ctrl-C with the popup open leaves no popup on the screen
start
T send-keys 'git '
waitfor $top
T send-keys C-c
sleep 0.4
! has $top; check 'Ctrl-C clears popup' $?
T send-keys X
sleep 0.3
has $'❯ git status\n*\n❯ X'; check 'Ctrl-C still aborts the line' $?

# 8. Up on an empty line walks history with no popup, even when the line matches ('ls --h')
start
T send-keys Up
waitfor '❯ ls --h'
sleep 0.3
! has $top; check 'history recall opens no popup' $?
T send-keys Up
waitfor '❯ echo hello world'
has '❯ echo hello world' && ! has $top; check 'second Up keeps walking history' $?

# 9. bottom of the screen: the prompt moves up so the popup fits
start
T send-keys 'seq 40' Enter
waitfor '40'
T send-keys 'git '
waitfor $bot
[[ "$(scr | sed '/^$/d' | tail -1)" == ${bot}* ]] && has '❯ git '; check 'popup fits at bottom' $?

# 10. resize: narrow and wide, popup never wider than the screen
start 80 24
T send-keys 'git '
waitfor $top
T resize-window -x 40 -y 24
sleep 0.4
maxw=0; for l in ${(M)${(f)"$(scr)"}:#[│╭╰]*}; do (( ${(m)#l} > maxw )) && maxw=${(m)#l}; done
(( maxw < 40 )) && has "❯ git status"$'\n'"$top"; check "resize to 40 cols, no key: rows redrawn ($maxw cols)" $?
T resize-window -x 120 -y 30
sleep 0.3
has "$top*$bot"; check 'resize to 120 cols: popup kept' $?

# 11. a 1,000 character paste is fast (bracketed paste and key by key)
start
long=${(l:1000::x:)}
T set-buffer -- "echo a$long "
t0=$EPOCHREALTIME
T paste-buffer -p
waitfor "x*$top"
t1=$EPOCHREALTIME
(( t1 - t0 < 0.5 )); check "bracketed paste of 1000 chars ($(( int((t1 - t0) * 1000) )) ms)" $?
start
t0=$EPOCHREALTIME
T send-keys -l -- "echo a$long"
T send-keys -l -- ' '
waitfor "xxx*$top"
t1=$EPOCHREALTIME
(( t1 - t0 < 3 )); check "typed 1000 chars ($(( int((t1 - t0) * 1000) )) ms)" $?

# 12. only palette colors 0-15, default colors, bold, dim and reverse in the popup
start
T send-keys 'git '
waitfor $top
sgr=( ${(u)${(M)${(ps:\e[:)"$(esc)"}:#[0-9;]#m*}%%m*} )
bad=( ${sgr:#(0|1|2|7|22|27|39|49|3[0-7]|9[0-7]|4[0-7]|10[0-7]|38;5;([0-9]|1[0-5]))} )
(( $#bad == 0 )); check "SGR codes: ${(j: :)sgr}" $?

# 13. vi mode: Esc with the popup closed still enters command mode
start 80 24 'bindkey -v'
T send-keys 'echo xhi'
sleep 0.2
T send-keys Escape
sleep 0.6
T send-keys x Enter
waitfor $'\nxh\n'
has $'\nxh\n'; check 'vi: Esc enters command mode' $?

T kill-server 2>/dev/null || true
print -r -- "$fails failed"
(( fails == 0 ))
