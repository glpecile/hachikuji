#!/usr/bin/env zsh
# Integration test: drives a real zsh in tmux with hachikuji, zsh-autosuggestions (async),
# a two-line starship prompt and zsh-syntax-highlighting, then reads the screen.
# Usage: zsh tests/zsh.zsh   (needs tmux and starship; clones zsh-autosuggestions once)
emulate -L zsh
setopt pipe_fail extended_glob
zmodload zsh/datetime zsh/mathfunc

root=${0:A:h:h}
bin=${HACHIKUJI_BIN:-$root/target/release/hachikuji}
# The hand-written git spec (specs/git.json): stable content, so the checks test the shell side.
idx=${HACHIKUJI_INDEX:-$root/build/m1.idx}
work=${TMPDIR:-/tmp}/hachikuji-it
mkdir -p $work
[[ -d $work/zsh-autosuggestions ]] ||
  git clone -q --depth 1 https://github.com/zsh-users/zsh-autosuggestions $work/zsh-autosuggestions
zsh_hl=${HK_ZSH_HL:-$(brew --prefix)/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh}
print -r -- 'add_newline = false
format = "$directory\n$character"' > $work/starship.toml

T() { tmux -L hk-it -f /dev/null "$@" }
fails=0
ok()   { print -r -- "ok   $1" }
fail() { print -r -- "FAIL $1"; (( ++fails )); print -r -- "$(scr)" | sed '/^$/d; s/^/     | /' }
check() { (( $2 == 0 )) && ok $1 || fail $1 }  # check name $?
scr() { T capture-pane -p }
esc() { T capture-pane -p -e }
has() { [[ "$(scr)" == *$~1* ]] }
waitfor() { local i; for i in {1..60}; do has "$1" && return 0; sleep 0.05; done; return 1 }
helper() { pgrep -f "^$bin serve" | tail -1 }
shellpid() { T display -p '#{pane_pid}' }

# start [cols] [rows] [extra zshrc lines]
start() {
  T kill-server 2>/dev/null || true
  rm -f $work/hist
  print -r -- "export LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 HACHIKUJI_INDEX=$idx
HISTFILE=$work/hist SAVEHIST=100 HISTSIZE=100
print -s 'git status'; print -s 'echo hello world'; print -s 'echo \$(git st'
$3
eval \"\$($bin init zsh)\"
source $work/zsh-autosuggestions/zsh-autosuggestions.zsh
export STARSHIP_CONFIG=$work/starship.toml
eval \"\$(starship init zsh)\"
source $zsh_hl" > $work/.zshrc
  T new-session -d -x ${1:-90} -y ${2:-24} -e ZDOTDIR=$work -c $root zsh -i
  waitfor '❯'
}

top='╭──'
bot='╰─'

# 1. popup opens with the ghost text before it, on a two-line prompt
start
T send-keys 'git '
waitfor "$top*status"
has "❯ git status*${top}*status*${bot}"; check 'popup opens, ghost text kept' $?
[[ "$(esc)" == *$'\e[32mgit'* ]]; check 'zsh-syntax-highlighting still colors the command' $?
[[ "$(esc | grep -aF $'\e[7m')" == *status* ]]; check 'first row selected (reverse)' $?

# 2. Down moves the selection; Tab accepts and opens the popup for the next token
T send-keys Down
sleep 0.2
[[ "$(esc | grep -aF $'\e[7m')" == *add* ]]; check 'Down moves selection' $?
start
T send-keys 'git ch'
waitfor "$top*checkout"
T send-keys Tab
waitfor "❯ git checkout*$top*main"
has "❯ git checkout*$top*main"; check 'Tab inserts item + space, popup lists branches' $?

# 3. Enter without moving runs the line and leaves no popup
start
T send-keys 'git status -s'
waitfor "$top*--short"
T send-keys Enter
sleep 0.5
has '❯ git status -s' && ! has $top; check 'Enter runs line when not moved, no popup left' $?

# 4. Enter after a move accepts the item
start
T send-keys 'git st'
waitfor "$top*stash"
T send-keys C-n Enter
waitfor "❯ git stash*$top*pop"
has "❯ git stash*$top*pop"; check 'Enter accepts after a move' $?

# 5. Esc closes the popup (KEYTIMEOUT delay)
start
T send-keys 'git '
waitfor $top
T send-keys Escape
sleep 0.6
! has $top; check 'Esc closes popup' $?

# 6. Right arrow accepts the ghost text only
start
T send-keys 'git '
waitfor "git status*$top"
T send-keys Right
sleep 0.3
has '❯ git status'; r=$?  # the popup may open for "status": that is right
T send-keys Enter
waitfor 'On branch'
(( r == 0 )) && has 'On branch' && ! has 'not found'; check 'Right accepts ghost only' $?

# 7. Ctrl-C clears the popup, aborts the line, and does not kill the helper
start
T send-keys 'git '
waitfor $top
h=$(helper)
T send-keys C-c
sleep 0.4
! has $top; check 'Ctrl-C clears popup' $?
T send-keys X
sleep 0.3
has $'❯ git status\n*\n❯ X'; check 'Ctrl-C still aborts the line' $?
[[ -n $h ]] && kill -0 $h 2>/dev/null; check "Ctrl-C does not kill the helper (pid $h)" $?

# 8. Up walks history with no popup, even when the line matches
start
T send-keys Up
waitfor '❯ echo \$\(git st'
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

# 10. resize with no key press: the helper renders again for the new width
start 90 24
T send-keys 'git '
waitfor $top
T resize-window -x 40 -y 24
sleep 0.4
maxw=0; for l in ${(M)${(f)"$(scr)"}:#[│╭╰]*}; do (( ${(m)#l} > maxw )) && maxw=${(m)#l}; done
(( maxw < 40 )) && has "❯ git status"$'\n'"$top"; check "resize to 40 cols, no key: rows redrawn ($maxw cols)" $?

# 11. a 1,000 character paste is fast
start
long=${(l:1000::x:)}
T set-buffer -- "git add $long"
t0=$EPOCHREALTIME
T paste-buffer -p
waitfor "xxx"$'\n'
t1=$EPOCHREALTIME
(( t1 - t0 < 0.5 )); check "bracketed paste of 1000 chars ($(( int((t1 - t0) * 1000) )) ms)" $?

# 12. only palette colors 0-15, default colors, bold, dim and reverse
start
T send-keys 'git '
waitfor $top
sgr=( ${(u)${(M)${(ps:\e[:)"$(esc)"}:#[0-9;]#m*}%%m*} )
bad=( ${sgr:#(0|1|2|7|22|27|39|49|3[0-7]|9[0-7]|4[0-7]|10[0-7]|38;5;([0-9]|1[0-5]))} )
(( $#sgr > 3 && $#bad == 0 )); check "SGR codes: ${(j: :)sgr}" $?

# 13. vi mode: Esc with the popup closed still enters command mode
start 90 24 'bindkey -v'
T send-keys 'echo xhi'
sleep 0.2
T send-keys Escape
sleep 0.6
T send-keys x Enter
waitfor $'\nxh\n'
has $'\nxh\n'; check 'vi: Esc enters command mode' $?

# 14. the shell stays the foreground process; its folder is what Herdr and Ghostty read
start
T send-keys 'git '
waitfor $top
T send-keys C-u 'cd /tmp' Enter
sleep 0.4
zpid=$(shellpid)
tty=$(T display -p '#{pane_tty}')
fg=$(ps -o tpgid= -p $zpid | tr -d ' ')
cwd=$(lsof -a -p $fg -d cwd -Fn 2>/dev/null | sed -n 's/^n//p')
h=$(helper)
hsess=$(ps -o sess=,tt= -p $h)
[[ $fg == $zpid && $cwd == /private/tmp && $hsess == *'??'* ]]
check "after cd: foreground is zsh ($fg), its cwd is $cwd, helper has no tty" $?

# 15. a stopped helper never blocks the prompt; it is killed after 1 s with no popup
start
T send-keys 'git '
waitfor $top
h=$(helper)
kill -STOP $h
T send-keys C-u 'git s'
sleep 0.3
has '❯ git s'; check 'typing works while the helper is stopped' $?
sleep 1
T send-keys 't'
sleep 0.3
has '❯ git st' && ! has $top && ! kill -0 $h 2>/dev/null; check 'hung helper killed, no popup' $?
kill -CONT $h 2>/dev/null

# 16. the helper exits with the shell
start
T send-keys 'git '
waitfor $top
h=$(helper)
T send-keys C-u 'exit' Enter
sleep 0.5
! kill -0 $h 2>/dev/null; check 'helper exits when the shell exits' $?

# 17. no orphan helper when the shell dies right after the first key
before=( $(pgrep -f "^$bin serve") )
for i in {1..5}; do
  start
  T send-keys 'g'
  T kill-server
done
sleep 1
left=( ${$(pgrep -f "^$bin serve"):|before} )
(( $#left == 0 )); check "no orphan helpers after the shell is killed ($#left left)" $?

# 18. no corrupt screen: type real lines key by key with the full Fig index (its descriptions
# have emoji and odd widths). After each line the screen must equal a full redraw (Ctrl-L):
# if ZLE's idea of the screen drifts, old text stays behind and the two differ.
if [[ -f $root/build/specs.idx ]]; then
  start 120 40 "export HACHIKUJI_INDEX=$root/build/specs.idx"
  bad=0
  for line in ${(f)"$(<$root/bench/commands.txt)"}; do
    T send-keys C-u
    sleep 0.2
    for (( i = 1; i <= $#line; i++ )); do T send-keys -l -- "$line[i]"; sleep 0.03; done
    sleep 0.5
    a=$(scr | sed '/^$/d' | sed -n '/❯/,$p')
    T send-keys C-l
    sleep 0.4
    b=$(scr | sed '/^$/d' | sed -n '/❯/,$p')
    [[ $a == $b ]] || { (( ++bad )); print -r -- "     differs after: $line" }
  done
  (( bad == 0 )); check "typed key by key: screen equals a full redraw ($bad of 18 lines differ)" $?
fi

# 19. an insertValue with {cursor}: the cursor goes between the quotes
start
T send-keys 'git commit --mes'
waitfor "$top*--message"
T send-keys Tab
sleep 0.3
T send-keys 'hi'
waitfor "❯ git commit -m 'hi'"
has "❯ git commit -m 'hi'"; check 'cursor goes between the quotes of -m' $?

T kill-server 2>/dev/null || true
print -r -- "$fails failed"
(( fails == 0 ))
