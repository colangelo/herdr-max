import sys, time
exec(open("/tmp/arena-p5/fix/bin/sizeprobe.py").read().split("q = C(FSOCK")[0])
q = C(FSOCK, "q")
def far(addr):
    r = q.call("terminal.get", {"terminal": addr})["result"]; return f"{r['cols']}x{r['rows']}"
A = C(FSOCK, "A"); A.attach(sys.argv[1], 100, 30); A.send(DATA, b"a"); time.sleep(0.5)
B = C(LSOCK, "B"); B.attach(sys.argv[2], 160, 50, resumed=True); time.sleep(0.5)
B.send(FOCUS, b"\x01"); time.sleep(0.6); log("after FOCUS in:", far(sys.argv[1]))
B.send(FOCUS, b"\x00"); time.sleep(0.6); log("after FOCUS out:", far(sys.argv[1]))
B.send(AUTO, b"\x1b[<0;10;5M"); time.sleep(0.6); log("after a mouse report as AUTO:", far(sys.argv[1]))
B.close(); A.close()
log("--- A (the far herdr) detaches while the view stays")
A = C(FSOCK, "A2"); A.attach(sys.argv[1], 100, 30); A.send(DATA, b"a"); time.sleep(0.5)
B = C(LSOCK, "B2"); B.attach(sys.argv[2], 160, 50, resumed=True); time.sleep(0.5); B.send(AUTO, b"x"); time.sleep(0.5)
A.close(); time.sleep(1); log("A gone, B stays: far", far(sys.argv[1]))
B.send(AUTO, b"y"); time.sleep(0.6); log("B AUTO input with A gone: far", far(sys.argv[1]))
A = C(FSOCK, "A3"); A.attach(sys.argv[1], 100, 30); time.sleep(0.6); log("A back (attach, no key): far", far(sys.argv[1]))
B.close(); A.close(); time.sleep(0.6); log("both gone: far", far(sys.argv[1]))
