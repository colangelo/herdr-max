"""One plain-mode JSON-RPC call to the scratch core: rpc.py <method> '<params json>'."""
import json, os, socket, sys

s = socket.socket(socket.AF_UNIX)
s.connect(os.path.join(os.environ["P5_ROOT"], "run/herdr-core", os.environ["HC_SESSION"] + ".sock"))
req = {"jsonrpc": "2.0", "id": 1, "method": sys.argv[1], "params": json.loads(sys.argv[2])}
s.sendall((json.dumps(req) + "\n").encode())
print(s.makefile("rb").readline().decode().rstrip())
