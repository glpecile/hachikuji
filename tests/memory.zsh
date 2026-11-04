#!/usr/bin/env zsh
# Memory check: starts 12 shells, types in each, and reports the private memory of each helper
# (macOS `footprint`: dirty pages; the mmap index is clean and shared, so it does not count).
# Usage: zsh tests/memory.zsh   (run tests/zsh.zsh once first: it sets up the plugins)
emulate -L zsh
root=${0:A:h:h}
bin=${HACHIKUJI_BIN:-$root/target/release/hachikuji}
idx=${HACHIKUJI_INDEX:-$root/build/specs.idx}
[[ -f $idx ]] || idx=$root/build/m1.idx
work=${TMPDIR:-/tmp}/hachikuji-mem
mkdir -p $work
print -r -- "export LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 HACHIKUJI_INDEX=$idx
PS1='%~ %# '
eval \"\$($bin init zsh)\"" > $work/.zshrc

T() { tmux -L hk-mem -f /dev/null "$@" }
T kill-server 2>/dev/null
before=( $(pgrep -f "^$bin serve") )
T new-session -d -x 100 -y 30 -e ZDOTDIR=$work -c $root zsh -i
for i in {1..11}; do T new-window -e ZDOTDIR=$work -c $root zsh -i; done
sleep 2
# Type the same lines in every shell: static specs, files, folders and git generators.
for line in 'git checkout ' 'git add src/' 'ls -' 'docker run --' 'npm install --save-' 'cd ~/'; do
  for i in {0..11}; do T send-keys -t :$i C-u $line; done
  sleep 0.5
done
sleep 1
total=0 max=0 n=0
for pid in ${$(pgrep -f "^$bin serve"):|before}; do
  kb=$(footprint $pid 2>/dev/null | sed -n 's/.*Footprint: \([0-9.]*\) \([KM]B\).*/\1 \2/p')
  [[ ${kb#* } == MB ]] && (( k = ${kb% *} * 1024 )) || (( k = ${kb% *} ))
  print -r -- "helper $pid: ${kb}"
  (( total += k, n++, k > max && (max = k) ))
done
T kill-server
printf 'helpers=%d  max=%.0f KB  mean=%.0f KB  total=%.1f MB\n' $n $max $(( n ? total / n : 0 )) $(( total / 1024. ))
(( n == 12 && max < 4096 ))
