import os, select, signal, struct, sys, termios, tty, fcntl, time
log = open(sys.argv[1], "a", buffering=1); last = [None]
def size():
    r, c = struct.unpack("hh", fcntl.ioctl(0, termios.TIOCGWINSZ, b"\0" * 4))
    if (c, r) != last[0]: last[0] = (c, r); log.write(f"{time.time():.3f} SIZE {c}x{r}\n")
signal.signal(signal.SIGWINCH, lambda *a: None); tty.setraw(0)
while True:
    size()
    try: rd, _, _ = select.select([0], [], [], 0.05)
    except InterruptedError: continue
    if rd:
        b = os.read(0, 1024)
        if not b: break
        log.write(f"{time.time():.3f} IN {b!r}\n")
