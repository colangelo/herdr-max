"""A `hc machine add --via` command that logs link traffic and can refuse to dial.

The remote side is a second core on this Mac under scratch XDG directories in $P5_ROOT.
While $P5_ROOT/dormant exists, it exits at once without starting anything (the gate of design D2).
"""
import os, subprocess, sys, threading, time

root = os.environ["P5_ROOT"]
line = sys.argv[-1]
log = open(os.path.join(root, "relay.log"), "a", buffering=1)
t0 = time.time()
log.write(f"{t0:.3f} START\n")
if os.path.exists(os.path.join(root, "dormant")):
    log.write(f"{t0:.3f} REFUSED\n")
    sys.exit(255)
env = dict(os.environ, XDG_STATE_HOME=f"{root}/rstate", XDG_RUNTIME_DIR=f"{root}/rrun",
           XDG_CONFIG_HOME=f"{root}/config")
env.pop("HC_SESSION", None)
p = subprocess.Popen(["sh", "-c", line], stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env, bufsize=0)


def pump(src, dst, tag):
    while True:
        b = os.read(src.fileno(), 65536)
        if not b:
            log.write(f"{time.time() - t0:9.3f} {tag} EOF\n")
            break
        log.write(f"{time.time() - t0:9.3f} {tag} {len(b)}\n")
        os.write(dst.fileno(), b)
    try:
        dst.close()
    except OSError:
        pass


threading.Thread(target=pump, args=(sys.stdin.buffer, p.stdin, "L->R"), daemon=True).start()
out = threading.Thread(target=pump, args=(p.stdout, sys.stdout.buffer, "R->L"), daemon=True)
out.start()
rc = p.wait()
out.join(2)
log.write(f"{time.time() - t0:9.3f} EXIT {rc}\n")
