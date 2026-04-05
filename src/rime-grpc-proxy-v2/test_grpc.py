import subprocess
import time
import os

env = {
    **os.environ,
    "LD_PRELOAD": "/opt/sogou/src/rime-grpc-proxy-v2/build/librime-grpc-proxy-v2.so",
    "ARIFY_ENGINES": "/opt/sogou/arif/build/src/.libs/libarif_rime.so:arif_rime_engine",
    "ARIFY_FRONTEND": "readline",
    "ARIFY_RL_NO_AUTO_UNSETENV": "1",
    "ARIF_RIME_MODULES": "default,grpc_proxy_v2",
    "ARIF_RIME_USER_DATA_DIR": "/opt/sogou/.cache/rime-grpc-v2-user",
    "ARIF_RIME_SHARED_DATA_DIR": "/usr/share/rime-data",
}

p = subprocess.Popen(
    ["/opt/sogou/arif/build/src/arify", "-p", "/opt/sogou/arif/build/src/.libs/libarify.so", "-f", "readline", "--", "bash", "--noprofile", "--norc"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    env=env,
    text=True
)

def read_until(p, s, timeout=2):
    t0 = time.time()
    out = ""
    while time.time() - t0 < timeout:
        c = p.stdout.read(1)
        if not c:
            break
        out += c
        if s in out:
            print(f"WAIT OK: {out}")
            return True
    print(f"WAIT TIMEOUT: {out}")
    return False

# We won't block forever
os.set_blocking(p.stdout.fileno(), False)

time.sleep(1)
p.stdin.write("bind '\"\\C-x\\C-a\": arify-toggle'\n")
p.stdin.flush()
time.sleep(1)
p.stdin.write("\x18\x01") # ^X^A
p.stdin.flush()

time.sleep(1)
p.stdin.write("ni\t\t\n")
p.stdin.flush()

time.sleep(3)
p.stdin.write("exit\n")
p.stdin.flush()

print(p.stdout.read() or "No output")
