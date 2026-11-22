"""Loopback-only SSH server (from candidate-4's): one accepted client key; exec requests run through sh -c in the far
scratch environment with stdin/stdout on the channel and stderr as the channel's stderr. Logs AUTH and EXEC."""
import os, socket, subprocess, sys, threading, time
import paramiko
FIX = "/tmp/arena-p5/fix/h"
LOG = open(f"{FIX}/ssh/auth.log", "a", buffering=1)
client_key = paramiko.Ed25519Key.from_private_key_file(f"{FIX}/ssh/ckey")
host_key = paramiko.RSAKey.generate(2048)
ENV = {"HOME": os.environ["HOME"], "USER": os.environ.get("USER", ""), "PATH": "/usr/bin:/bin:" + os.environ["HOME"] + "/.local/bin",
       "XDG_STATE_HOME": f"{FIX}/rstate", "XDG_RUNTIME_DIR": f"{FIX}/rrun", "XDG_CONFIG_HOME": f"{FIX}/rconfig"}
class Server(paramiko.ServerInterface):
    def check_auth_publickey(self, username, key):
        ok = key.asbytes() == client_key.asbytes(); LOG.write(f"{time.time():.3f} AUTH ok={ok}\n")
        return paramiko.AUTH_SUCCESSFUL if ok else paramiko.AUTH_FAILED
    def get_allowed_auths(self, username): return "publickey"
    def check_channel_request(self, kind, chanid):
        return paramiko.OPEN_SUCCEEDED if kind == "session" else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED
    def check_channel_exec_request(self, channel, command):
        LOG.write(f"{time.time():.3f} EXEC\n")
        proc = subprocess.Popen(["sh", "-c", command.decode()], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=ENV)
        def out():
            while (b := os.read(proc.stdout.fileno(), 65536)): channel.sendall(b)
            channel.send_exit_status(proc.wait()); channel.close()
        def err():
            while (b := os.read(proc.stderr.fileno(), 65536)): channel.sendall_stderr(b)
        def inp():
            try:
                while (b := channel.recv(65536)): proc.stdin.write(b); proc.stdin.flush()
            except Exception: pass
            try: proc.stdin.close()
            except Exception: pass
        for f in (out, err, inp): threading.Thread(target=f, daemon=True).start()
        return True
def handle(sock):
    t = paramiko.Transport(sock); t.add_server_key(host_key); t.start_server(server=Server())
    while t.is_active(): time.sleep(0.5)
ls = socket.socket(); ls.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); ls.bind(("127.0.0.1", int(sys.argv[1]))); ls.listen(8)
LOG.write(f"{time.time():.3f} LISTEN\n")
while True:
    s, _ = ls.accept(); threading.Thread(target=handle, args=(s,), daemon=True).start()
