import os
import socket
import time

PORT = int(os.environ.get("POC_HOST_PORT", "22702"))


def recv_line(f):
    return f.readline().decode("utf-8", "replace").rstrip("\r\n")


def send_cmd(f, cmd):
    f.write((cmd + "\n").encode())
    return recv_line(f)


def main():
    sock = None
    for _ in range(300):
        try:
            sock = socket.create_connection(("127.0.0.1", PORT), timeout=0.5)
            break
        except OSError:
            time.sleep(0.05)

    if sock is None:
        print(f"connect failed port={PORT}")
        return 1

    sock.settimeout(20.0)
    f = sock.makefile("rwb", buffering=0)

    print("HELLO", recv_line(f))
    print("STATUS =>", send_cmd(f, "STATUS"))
    print("ACTIVATE =>", send_cmd(f, "ACTIVATE"))
    print("CP 936 =>", send_cmd(f, "CP 936"))

    print("TRACEU nihao =>", send_cmd(f, "TRACEU nihao"))
    print("PREEDIT =>", send_cmd(f, "PREEDIT"))
    print("CAND #1 =>", send_cmd(f, "CAND"))

    print("TRACEPIPEU nihao =>", send_cmd(f, "TRACEPIPEU nihao"))
    print("CAND #2 =>", send_cmd(f, "CAND"))
    print("CAND #3 =>", send_cmd(f, "CAND"))

    print("QUIT =>", send_cmd(f, "QUIT"))
    sock.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())