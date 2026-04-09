import os
import re
import socket
import time


PORT = int(os.environ.get("POC_HOST_PORT", "22702"))
TERMS = [
    t.strip()
    for t in os.environ.get("POC_S2T_CONV_TERMS", "后,发展,中国,龙,发,国").split(",")
    if t.strip()
]


def recv_line(f):
    return f.readline().decode("utf-8", "replace").rstrip("\r\n")


def send_cmd(f, cmd):
    f.write((cmd + "\n").encode())
    return recv_line(f)


def send_cmd_safe(f, cmd):
    try:
        line = send_cmd(f, cmd)
        print(f"{cmd} =>", line)
        return line
    except (OSError, TimeoutError) as e:
        print(f"{cmd} => <io_err:{type(e).__name__}:{e}>")
        return None


def conv_first_item(line):
    if not line:
        return ""
    m = re.search(r"items=\[(.*?)\]", line)
    if not m:
        return ""
    raw = m.group(1).strip()
    if not raw:
        return ""
    first = raw.split("|", 1)[0].strip()
    if first == "...":
        return ""
    return first


def toggle_hotkey(f):
    line = send_cmd_safe(f, "KEYCHORD 46 1 1 0")
    return line is not None and not line.startswith("ERR")


def probe_state(f, state_name):
    out = {}

    if send_cmd_safe(f, "ACTIVATE") is None:
        return None

    for term in TERMS:
        conv_line = send_cmd_safe(f, f"CONV {term}")
        if conv_line is None:
            return None
        first = conv_first_item(conv_line)
        out[term] = {
            "first": first,
            "line": conv_line,
        }
        print(
            "S2T_CONV_SAMPLE"
            f" state={state_name}"
            f" term={term}"
            f" first=[{first}]"
            f" line=[{conv_line}]"
        )
        time.sleep(0.08)

    return out


def main():
    sock = None
    for _ in range(400):
        try:
            sock = socket.create_connection(("127.0.0.1", PORT), timeout=0.5)
            break
        except OSError:
            time.sleep(0.05)

    if sock is None:
        print(f"connect failed port={PORT}")
        return 1

    sock.settimeout(30.0)
    f = sock.makefile("rwb", buffering=0)

    try:
        print("HELLO", recv_line(f))
    except (OSError, TimeoutError) as e:
        print(f"S2T_CONV_FAIL reason=hello_failed detail={type(e).__name__}:{e}")
        sock.close()
        return 1

    if send_cmd_safe(f, "STATUS") is None:
        sock.close()
        return 1
    if send_cmd_safe(f, "ACTIVATE") is None:
        sock.close()
        return 1
    if send_cmd_safe(f, "CP 936") is None:
        sock.close()
        return 1

    before = probe_state(f, "before")
    if before is None:
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 2

    if not toggle_hotkey(f):
        print("S2T_CONV_FAIL reason=keychord_unavailable")
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 3

    after_once = probe_state(f, "after_once")
    if after_once is None:
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 3

    if not toggle_hotkey(f):
        print("S2T_CONV_FAIL reason=keychord_second_failed")
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 4

    after_twice = probe_state(f, "after_twice")
    if after_twice is None:
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 4

    changed = []
    restored = []
    compared = []

    for term in TERMS:
        b = before[term]["first"]
        a1 = after_once[term]["first"]
        a2 = after_twice[term]["first"]
        if b and a1:
            compared.append(term)
            if b != a1:
                changed.append(term)
                if a2 == b:
                    restored.append(term)

    if changed and len(restored) == len(changed):
        print(
            "S2T_CONV_PASS"
            f" terms={','.join(TERMS)}"
            f" compared={','.join(compared)}"
            f" changed={','.join(changed)}"
            f" restored={','.join(restored)}"
        )
    else:
        print(
            "S2T_CONV_OBS"
            f" terms={','.join(TERMS)}"
            f" compared={','.join(compared)}"
            f" changed={','.join(changed)}"
            f" restored={','.join(restored)}"
        )

    send_cmd_safe(f, "QUIT")
    sock.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())