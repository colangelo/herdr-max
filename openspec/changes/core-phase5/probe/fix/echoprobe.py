"""F5: a view attached `resumed`, typing only AUTOMATIC_INPUT, that echoes each size event as a RESIZE so its
recorded size stays the far one. Does the echo apply, storm, or keep the far size right when the far herdr leaves?"""
import json, socket, sys, threading, time
exec(open("/tmp/arena-p5/fix/bin/sizeprobe.py").read().split("q = C(FSOCK")[0])
q = C(FSOCK, "q")
def far(addr):
    r = q.call("terminal.get", {"terminal": addr})["result"]; return f"{r['cols']}x{r['rows']}"
FA, LA = sys.argv[1], sys.argv[2]
A = C(FSOCK, "A"); A.attach(FA, 100, 30); A.send(DATA, b"a"); time.sleep(0.5)
B = C(LSOCK, "B"); g = q.call("terminal.get", {"terminal": FA})["result"]; B.attach(LA, g["cols"], g["rows"], resumed=True)
ev = socket.socket(socket.AF_UNIX); ev.connect(LSOCK); evf = ev.makefile("rb")
ev.sendall((json.dumps({"jsonrpc": "2.0", "id": 1, "method": "events.subscribe", "params": {"terminal": LA}}) + "\n").encode())
evf.readline(); n = [0]; last = [None]
def rd():
    for line in evf:
        p = json.loads(line).get("params", {})
        if p.get("type") == "size":
            n[0] += 1; d = p["data"]
            if (d["cols"], d["rows"]) != last[0]: last[0] = (d["cols"], d["rows"]); B.resize(d["cols"], d["rows"])
threading.Thread(target=rd, daemon=True).start()
for c, r in [(90, 25), (120, 40)]:
    A.resize(c, r); time.sleep(1); B.send(AUTO, b"k"); time.sleep(0.5); log(f"A -> {c}x{r}: far", far(FA), "size events so far", n[0])
A.close(); time.sleep(1); log("A gone: far", far(FA), "(want 120x40, the last far size)", "size events", n[0])
B.send(AUTO, b"k"); time.sleep(0.5); log("B types with A gone: far", far(FA))
A = C(FSOCK, "A2"); A.attach(FA, 100, 30); time.sleep(0.8); log("A back at 100x30: far", far(FA), "size events", n[0])
time.sleep(3); log("3 s idle: size events", n[0], "(no storm)")
B.close(); A.close()
