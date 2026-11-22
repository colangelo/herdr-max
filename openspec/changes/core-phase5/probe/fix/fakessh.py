"""Stand-in for ssh (one process owning the link, as OpenSSH is): runs the remote command in the far environment
behind pipes it pumps, so its death closes core's pipes as a dropped ssh would. Modes from h/ssh_mode."""
import os, signal, subprocess, sys, threading, time
FIX = "/tmp/arena-p5/fix/h"
remote = sys.argv[-1]
mode = open(f"{FIX}/ssh_mode").read().strip() if os.path.exists(f"{FIX}/ssh_mode") else "relay"
with open(f"{FIX}/ssh.log", "a") as f: f.write(f"{time.time():.3f} SSH pid={os.getpid()} mode={mode}\n")
if mode == "hang": time.sleep(3600); sys.exit(255)
if mode == "banner": sys.stderr.write("Tailscale SSH requires an additional check.\n"); sys.stderr.flush(); time.sleep(3600)
if mode == "slow": time.sleep(5)
env = {"HOME": os.environ["HOME"], "USER": os.environ.get("USER", ""), "PATH": "/usr/bin:/bin:" + os.environ["HOME"] + "/.local/bin",
       "XDG_STATE_HOME": f"{FIX}/rstate", "XDG_RUNTIME_DIR": f"{FIX}/rrun", "XDG_CONFIG_HOME": f"{FIX}/rconfig"}
p = subprocess.Popen(["sh", "-c", remote], stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env, bufsize=0, start_new_session=True)
def die(*_):
    try: os.killpg(p.pid, signal.SIGKILL)
    except ProcessLookupError: pass
    os._exit(255)
signal.signal(signal.SIGTERM, die)
def pump(src, dst):
    while True:
        b = os.read(src, 65536)
        if not b: break
        os.write(dst, b)
threading.Thread(target=pump, args=(0, p.stdin.fileno()), daemon=True).start()
out = threading.Thread(target=pump, args=(p.stdout.fileno(), 1), daemon=True); out.start()
rc = p.wait(); out.join(1); sys.exit(rc)
