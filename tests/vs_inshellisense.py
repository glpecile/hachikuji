#!/usr/bin/env python3
"""Compares hachikuji, inshellisense and plain zsh: time to the first prompt, and key press to the last
output byte (p99). Same zshrc for each (zsh-syntax-highlighting); only the plugin changes.
Uses the installed hachikuji (~/.local) and inshellisense (Homebrew).
Usage: python3 tests/vs_inshellisense.py <hk|is|base> [keys]"""
import fcntl, os, pty, select, struct, sys, tempfile, termios, time, statistics

root = "/Users/glpecile/Developer/hachikuji"
mode, n_keys = sys.argv[1], int(sys.argv[2]) if len(sys.argv) > 2 else 300
hl = "/opt/homebrew/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh"
plug = {
    "hk": f"HACHIKUJI_BIN={os.path.expanduser('~/.local/bin/hachikuji')} source ~/.local/share/hachikuji/hachikuji.zsh",
    "is": "source ~/.local/share/inshellisense/init/zsh/init.zsh",
    "base": "",
}[mode]
zdot = tempfile.mkdtemp()
open(f"{zdot}/.zshrc", "w").write(f"""export LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8
PS1='READY%# '
{plug}
source {hl}
""")
env = {**os.environ, "ZDOTDIR": zdot, "TERM": "xterm-256color", "PATH": "/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"}
for k in ("ISTERM", "HACHIKUJI_INDEX", "HACHIKUJI_NO_GENERATORS"):
    env.pop(k, None)
lines = [l for l in open(f"{root}/bench/commands.txt").read().splitlines() if l]


def spawn():
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(root)
        os.execve("/bin/zsh", ["zsh", "-i"], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
    return pid, fd


def drain(fd, quiet):
    last, out = None, b""
    while True:
        r, _, _ = select.select([fd], [], [], quiet)
        if not r:
            return last, out
        try:
            d = os.read(fd, 65536)
        except OSError:
            return last, out
        if not d:
            return last, out
        out += d
        last = time.perf_counter()


def kill(pid, fd):
    try:
        os.kill(-os.getpgid(pid), 9)
    except Exception:
        pass
    try:
        os.kill(pid, 9)
    except Exception:
        pass
    os.close(fd)
    os.waitpid(pid, 0)


# Startup: fork to first "READY" byte.
starts = []
for _ in range(10):
    t0 = time.perf_counter()
    pid, fd = spawn()
    buf = b""
    while b"READY" not in buf and time.perf_counter() - t0 < 10:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try:
                buf += os.read(fd, 65536)
            except OSError:
                break
    starts.append((time.perf_counter() - t0) * 1000)
    kill(pid, fd)

# Latency: time from key to the last output byte (quiet 150 ms).
pid, fd = spawn()
drain(fd, 1.5)
times, done, seen = [], 0, b""
while done < n_keys:
    for line in lines:
        for ch in line:
            t0 = time.perf_counter()
            os.write(fd, ch.encode())
            last, out = drain(fd, 0.15)
            seen += out
            if last is not None:
                times.append((last - t0) * 1000)
            done += 1
        os.write(fd, b"\x15")
        drain(fd, 0.2)
        if done >= n_keys:
            break
kill(pid, fd)
times.sort()
p = lambda q: times[int((len(times) - 1) * q)]
print(f"{mode}: startup median={statistics.median(starts):.0f}ms | keys={len(times)} p50={p(.5):.1f}ms p99={p(.99):.1f}ms max={times[-1]:.1f}ms | popup text seen: {b'checkout' in seen}")
