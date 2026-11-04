#!/usr/bin/env python3
"""Startup cost (hard constraint 5): what the line from `hachikuji init zsh` adds to zsh startup,
up to the first prompt (it includes the one-time precmd setup). Timed inside zsh with
EPOCHREALTIME, from just before the line to the end of precmd, with and without the line.

Usage: python3 tests/startup.py [runs]     (default 30)
"""
import os, pty, select, statistics, subprocess, sys, tempfile, time

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
bin_ = os.environ.get("HACHIKUJI_BIN", f"{root}/target/release/hachikuji")
runs = int(sys.argv[1]) if len(sys.argv) > 1 else 30
line = subprocess.run([bin_, "init", "zsh"], capture_output=True, text=True).stdout


def first_prompt(rc):
    zdot = tempfile.mkdtemp()
    with open(f"{zdot}/.zshrc", "w") as f:
        f.write("zmodload zsh/datetime; _t0=$EPOCHREALTIME\n" + rc)
        f.write("_t_end() { print -r -- \"HKT=$(( (EPOCHREALTIME - _t0) * 1000 ))=\" }\n")
        f.write("precmd_functions+=(_t_end)\nPS1='READY> '\n")
    pid, fd = pty.fork()
    if pid == 0:
        os.execve("/bin/zsh", ["zsh", "-i"], {**os.environ, "ZDOTDIR": zdot})
    out = b""
    while b"READY> " not in out:
        r, _, _ = select.select([fd], [], [], 5)
        if not r:
            break
        out += os.read(fd, 4096)
    t = float(out.split(b"HKT=")[1].split(b"=")[0])
    os.kill(pid, 9)
    os.waitpid(pid, 0)
    os.close(fd)
    return t


first_prompt(line)  # warm-up: page cache, dyld
a, b = [], []
for _ in range(runs):  # interleave, so drift hits both sides alike
    a.append(first_prompt(""))
    b.append(first_prompt(line))
base, with_hk = statistics.median(a), statistics.median(b)
print(f"rc to first prompt (median of {runs}): without {base:.2f} ms, with hachikuji {with_hk:.2f} ms, added {with_hk - base:.2f} ms")
sys.exit(0 if with_hk - base < 10 else 1)
