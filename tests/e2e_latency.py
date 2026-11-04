#!/usr/bin/env python3
"""End-to-end latency (number 4b): key press to updated popup, through a real zsh in a pty.

For each key: write it, then read the pty until it is quiet for 40 ms. The latency is the time
of the last output byte: ZLE has then drawn the line and the popup from the helper's response.
Only hachikuji and zsh-syntax-highlighting are loaded, so no other plugin writes late output.
Generators are off (HACHIKUJI_NO_GENERATORS): the target is for static specs, and an async
generator update would count as late output. Set HK_E2E_GENERATORS=1 to include them.

Usage: python3 tests/e2e_latency.py [keys]     (default 600 keys)
"""
import fcntl, os, pty, select, struct, sys, tempfile, termios, time

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
bin_ = os.environ.get("HACHIKUJI_BIN", f"{root}/target/release/hachikuji")
idx = os.environ.get("HACHIKUJI_INDEX", f"{root}/build/specs.idx")
if not os.path.exists(idx):
    idx = f"{root}/build/m1.idx"
hl = "/opt/homebrew/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh"
n_keys = int(sys.argv[1]) if len(sys.argv) > 1 else 600
lines = [l for l in open(f"{root}/bench/commands.txt").read().splitlines() if l]

zdot = tempfile.mkdtemp()
with open(f"{zdot}/.zshrc", "w") as f:
    f.write(f"""export LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 HACHIKUJI_INDEX={idx}
PS1='%~ %# '
[[ -z $HK_E2E_NO_HK ]] && eval "$({bin_} init zsh)"
[[ -z $HK_E2E_NO_HL && -f {hl} ]] && source {hl}
""")

env = {**os.environ, "ZDOTDIR": zdot, "TERM": "xterm-256color"}
if not os.environ.get("HK_E2E_GENERATORS"):
    env["HACHIKUJI_NO_GENERATORS"] = "1"
pid, fd = pty.fork()
if pid == 0:
    os.chdir(root)
    os.execve("/bin/zsh", ["zsh", "-i"], env)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))


def drain(quiet):
    """Reads until no output for `quiet` seconds. Returns the time of the last byte."""
    last = None
    while True:
        r, _, _ = select.select([fd], [], [], quiet)
        if not r:
            return last
        try:
            if not os.read(fd, 65536):
                return last  # EOF: zsh exited
        except OSError:
            return last
        last = time.perf_counter()


drain(1.0)  # prompt
times = []
done = 0
while done < n_keys:
    for line in lines:
        for ch in line:
            t0 = time.perf_counter()
            os.write(fd, ch.encode())
            last = drain(0.04)
            if last is not None:
                times.append((last - t0) * 1000)
            done += 1
        os.write(fd, b"\x15")  # Ctrl-U: next line
        drain(0.1)
        if done >= n_keys:
            break

os.write(fd, b"\x15exit\r")
drain(0.5)
try:
    os.kill(pid, 9)
except ProcessLookupError:
    pass
os.waitpid(pid, 0)
times.sort()
p = lambda q: times[int((len(times) - 1) * q)]
print(f"keys={len(times)} p50={p(0.5):.2f}ms p99={p(0.99):.2f}ms max={times[-1]:.2f}ms")
sys.exit(0 if p(0.99) < 16 else 1)
