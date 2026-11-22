"""F5: can a view type without taking the far terminal's size? A = the far herdr (direct on the far core),
B = the m4m view (through the local core's machine link)."""
import json, os, socket, struct, sys, threading, time
FIX = os.environ["FIX"]
FSOCK = f"{FIX}/rrun/herdr-core/p5fix-far.sock"; LSOCK = f"{FIX}/run/herdr-core/p5fix-loc.sock"
DATA, CREDIT, CLOSE, RESIZE, FOCUS, FACTS, AUTO = 0, 1, 2, 3, 4, 5, 6
T0 = time.time()
def log(*a): print(f"{time.time()-T0:7.3f}", *a, flush=True)
class C:
    def __init__(s, path, tag):
        s.tag = tag; s.s = socket.socket(socket.AF_UNIX); s.s.connect(path); s.f = s.s.makefile("rb")
        s.s.sendall((json.dumps({"jsonrpc": "2.0", "id": 0, "method": "hello", "params": {"framed": True}}) + "\n").encode())
        s.f.readline(); s.replies = {}; s.chan = None; s.lock = threading.Lock(); s.n = 100; s.resizes = []
        threading.Thread(target=s.reader, daemon=True).start()
    def frame(s, ch, k, p=b""):
        with s.lock: s.s.sendall(struct.pack("<IBI", ch, k, len(p)) + p)
    def call(s, m, p):
        s.n += 1; rid = s.n
        s.frame(0, DATA, json.dumps({"jsonrpc": "2.0", "id": rid, "method": m, "params": p}).encode())
        for _ in range(500):
            if rid in s.replies: return s.replies.pop(rid)
            time.sleep(0.01)
    def reader(s):
        while True:
            h = s.f.read(9)
            if len(h) < 9: return
            ch, k, n = struct.unpack("<IBI", h); p = s.f.read(n)
            if ch == 0:
                m = json.loads(p)
                if "id" in m: s.replies[m["id"]] = m
            elif k == DATA: s.frame(ch, CREDIT, struct.pack("<I", len(p)))
            elif k == RESIZE: s.resizes.append(struct.unpack("<HH", p))
    def attach(s, addr, cols, rows, **kw):
        r = s.call("terminal.attach", dict(terminal=addr, cols=cols, rows=rows, **kw))
        if "error" in r: log(s.tag, "attach error", r["error"]); return
        res = r["result"]; s.chan = res["channel"]
        log(s.tag, "attached", addr, f"{cols}x{rows}", kw, "automatic_input=", res.get("automatic_input"), "resize_notifications=", res.get("resize_notifications"))
    def send(s, kind, b): s.frame(s.chan, kind, b)
    def resize(s, c, r): s.frame(s.chan, RESIZE, struct.pack("<HH", c, r))
    def close(s): s.frame(s.chan, CLOSE); time.sleep(0.2); s.s.close()
q = C(FSOCK, "q")
def far(addr="t1"):
    r = q.call("terminal.get", {"terminal": addr})["result"]; return f"{r['cols']}x{r['rows']}"
FAR, LOC = sys.argv[1], sys.argv[2]
A = C(FSOCK, "A"); A.attach(FAR, 100, 30); A.send(DATA, b"a"); time.sleep(0.5); log("after A types: far", far(FAR))
for name, kw, kinds in [("observe", {"observe": True}, [DATA, AUTO]),
                        ("resumed+auto", {"resumed": True}, [AUTO, AUTO]),
                        ("resumed+data", {"resumed": True}, [DATA]),
                        ("plain", {}, [])]:
    B = C(LSOCK, "B:" + name); B.attach(LOC, 160, 50, **kw); time.sleep(0.6); log(name, "after attach: far", far(FAR))
    for i, k in enumerate(kinds):
        B.send(k, f"{name[0]}{i}".encode()); time.sleep(0.6); log(name, "after kind", k, "send: far", far(FAR))
    A.resize(90, 25); time.sleep(0.6); log(name, "A resized 90x25: far", far(FAR), "B got RESIZE frames", B.resizes[-3:])
    if kinds: B.send(kinds[-1], b"z"); time.sleep(0.6); log(name, "B sends again: far", far(FAR))
    B.close(); A.resize(100, 30); A.send(DATA, b"a"); time.sleep(0.5); log("reset: far", far(FAR))
