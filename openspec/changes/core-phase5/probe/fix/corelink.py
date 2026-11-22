"""Prototype of `herdr core-link <name>` (core-phase5 D2/D10 after the roundtable): lease and gate check, a
single-flight lock, a watcher process beside ssh, and exec of ssh itself so the via pid core holds IS ssh.
F2: own deadline 15 s, gate written before core's ~19.8 s kill. F3: flock held by ssh for the link's life.
F6: state from a fixed path (here a constant; in herdr the passwd home), not the starter's XDG.
F10: a far-side guard that refuses when the far core is not running, so the dial never starts one."""
import fcntl, json, os, select, signal, sys, time
NAME = sys.argv[1]
LINE = sys.argv[-1]
STATE = f"/tmp/arena-p5/fix/h/cm/{NAME}"          # fixed, independent of the core starter's env (F6)
SSH = "/tmp/arena-p5/fix/bin/fakessh.py"
FAR_SESSION = open(f"{STATE}/session").read().strip()
DEADLINE = float(os.environ.get("CL_DEADLINE", "15"))
MARK = "HERDR-LINK-UP"
log = open(f"{STATE}/link.log", "a", buffering=1)
def L(*a): log.write(f"{time.time():.3f} pid={os.getpid()} " + " ".join(map(str, a)) + "\n")
def leased():
    now = time.time(); d = f"{STATE}/leases"
    return [f for f in os.listdir(d) if not f.endswith(".new") and float(open(f"{d}/{f}").read()) > now]
def gate(cls):
    tmp = f"{STATE}/gate.json.new"; open(tmp, "w").write(json.dumps({"class": cls, "at": time.time()})); os.rename(tmp, f"{STATE}/gate.json")
L("START")
try:
    g = json.load(open(f"{STATE}/gate.json"))["class"]
except FileNotFoundError:
    g = "open"
if g != "open": L("REFUSE gate", g); sys.exit(75)
if not LINE.startswith("sh -c "): L("REFUSE not a posix line:", LINE[:12]); sys.exit(75)  # core's Windows fallback probe
if not leased(): L("REFUSE dormant"); sys.exit(75)
lock = os.open(f"{STATE}/dial.lock", os.O_RDWR | os.O_CREAT, 0o600)
t0 = time.time()
while True:
    try: fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB); break
    except BlockingIOError:
        if time.time() - t0 > 2: L("REFUSE busy"); sys.exit(75)
        time.sleep(0.05)
L("LOCKED after", f"{time.time()-t0:.3f}s")
os.set_inheritable(lock, True)
r, w = os.pipe()
if os.fork() == 0:
    # watcher: holds none of core's stdio, so ssh's exit is the link's end
    os.close(w); fd = os.open(os.devnull, os.O_RDWR); os.dup2(fd, 0); os.dup2(fd, 1); os.setsid()
    ssh = os.getppid(); up = False; start = time.time(); next_lease = start + 5; buf = b""
    def end(why, cls=None):
        if cls: gate(cls)
        L("END", why, "ssh alive" if os.getppid() == ssh else "ssh gone")
        if os.getppid() == ssh:
            os.kill(ssh, signal.SIGTERM)
            time.sleep(1)
            if os.getppid() == ssh: os.kill(ssh, signal.SIGKILL)
    while True:
        if os.getppid() != ssh: L("SSH GONE"); os._exit(0)
        rd, _, _ = select.select([r], [], [], 0.5)
        if rd:
            b = os.read(r, 4096)
            if not b: L("SSH EXITED"); os._exit(0)
            buf += b
            if not up and MARK.encode() in buf: up = True; L("UP after", f"{time.time()-start:.3f}s")
            if b"Tailscale SSH requires" in buf: end("tailscale check", "needs_check"); buf = b""
            if b"herdr-link: herdr-core is not running there" in buf: L("FAR CORE DOWN"); buf = b""
        now = time.time()
        if not up and now - start > DEADLINE: end("no far side after deadline", "auth_stalled"); up = True
        if now >= next_lease:
            next_lease = now + 5
            if not leased(): end("lease ended")
os.close(r); os.dup2(w, 2); os.close(w)
sockexpr = (f'd=${{XDG_RUNTIME_DIR:+$XDG_RUNTIME_DIR/herdr-core}}; [ -n "$d" ] || {{ [ -d /run/user/$(id -u) ] && d=/run/user/$(id -u)/herdr-core; }}; '
            f'[ -n "$d" ] || d=${{XDG_STATE_HOME:-$HOME/.local/state}}/herdr-core/{FAR_SESSION}; ')
guard = (f"sh -c '{sockexpr}nc -U -w 2 \"$d/{FAR_SESSION}.sock\" </dev/null >/dev/null 2>&1 || "
         f"{{ echo \"herdr-link: herdr-core is not running there\" >&2; exit 69; }}; echo {MARK} >&2'")
if os.path.exists("/tmp/arena-p5/fix/h/ssh_real"):   # real OpenSSH to the loopback server, D10 options after the roundtable
    k = "/tmp/arena-p5/fix/h/ssh/ckey"
    os.execv("/usr/bin/ssh", ["ssh", "-F", "/dev/null", "-p", "2298", "-i", k, "-o", "IdentitiesOnly=yes", "-o", "IdentityAgent=none",
        "-o", "BatchMode=yes", "-o", "NumberOfPasswordPrompts=0", "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null",
        "-o", "ConnectTimeout=10", "-o", "ConnectionAttempts=1", "-o", "ControlMaster=no", "-o", "ControlPath=none",
        "-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=2", "-o", "LogLevel=ERROR", "-T", "127.0.0.1", f"{guard} && {LINE}"])
os.execv(sys.executable, [sys.executable, "-I", SSH, "-T", "target", f"{guard} && {LINE}"])
