"""F5: how the view learns the far size without sending RESIZE: size events on the local core for loop/tN."""
import json, os, socket, sys, threading, time
exec(open("/tmp/arena-p5/fix/bin/sizeprobe.py").read().split("q = C(FSOCK")[0])
ev = socket.socket(socket.AF_UNIX); ev.connect(LSOCK); evf = ev.makefile("rb")
ev.sendall((json.dumps({"jsonrpc": "2.0", "id": 1, "method": "events.subscribe", "params": {"terminal": sys.argv[2]}}) + "\n").encode())
evf.readline(); mark = {"t": time.time()}; seen = []
def rd():
    for line in evf:
        m = json.loads(line); p = m.get("params", {})
        if p.get("type") == "size": seen.append((time.time() - mark["t"], p.get("data")))
threading.Thread(target=rd, daemon=True).start()
A = C(FSOCK, "A"); A.attach(sys.argv[1], 100, 30); A.send(DATA, b"a"); time.sleep(0.5)
B = C(LSOCK, "B"); B.attach(sys.argv[2], 160, 50, resumed=True)
for c, r in [(90, 25), (120, 40), (100, 30)]:
    seen.clear(); mark["t"] = time.time(); A.resize(c, r); time.sleep(1.5)
    log("A ->", f"{c}x{r}", "size events at the view:", [(round(t, 3), d) for t, d in seen])
B.send(AUTO, b"k"); time.sleep(1); log("events after the view's input (expect none):", seen[-2:] if seen else [])
