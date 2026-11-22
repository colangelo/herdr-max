import json, os, socket, sys, time
s = socket.socket(socket.AF_UNIX); s.settimeout(float(os.environ.get("RPC_TO", "60")))
s.connect(sys.argv[1])
t = time.time()
s.sendall((json.dumps({"jsonrpc": "2.0", "id": 1, "method": sys.argv[2], "params": json.loads(sys.argv[3])}) + "\n").encode())
l = s.makefile("rb").readline().decode().rstrip()
print(f"[{time.time()-t:.2f}s] {l[:int(os.environ.get('CUT', '400'))]}")
