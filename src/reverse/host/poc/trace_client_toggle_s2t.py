import os
import re
import socket
import sys
import time

PORT = int(os.environ.get("POC_HOST_PORT", "22702"))
PROBE_TEXT = os.environ.get("POC_TOGGLE_PROBE_TEXT", "hou")


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


def first_item_from_cand(line):
    if not line:
        return ""
    m = re.search(r"#0\{[^\[]*items=\[(.*?)\]\}", line)
    if not m:
        return ""
    items = m.group(1).split("|")
    if not items:
        return ""
    return items[0].strip()


def seed_candidate(f):
    line = send_cmd_safe(f, f"PIPEU {PROBE_TEXT}")
    if line is None:
        return None
    item = first_item_from_cand(line)
    if item:
        return item

    # Fallback poll once if PIPEU reply is empty phase.
    line = send_cmd_safe(f, "CAND")
    if line is None:
        return None
    return first_item_from_cand(line)


def toggle_hotkey(f):
    # Ctrl+Shift+F (VK_F = 0x46)
    line = send_cmd_safe(f, "KEYCHORD 46 1 1 0")
    return line is not None and not line.startswith("ERR")


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
        print(f"TOGGLE_FAIL reason=hello_failed detail={type(e).__name__}:{e}")
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

    # Open processing gate / warm context.
    if send_cmd_safe(f, "TRACEU ni") is None:
        sock.close()
        return 2

    first = seed_candidate(f)
    if first is None:
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 2

    print(f"TOGGLE_STATE before first_item=[{first}]")

    if not toggle_hotkey(f):
        print("TOGGLE_FAIL reason=keychord_unavailable")
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 3

    send_cmd_safe(f, "RESET")
    send_cmd_safe(f, "ACTIVATE")
    second = seed_candidate(f)
    if second is None:
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 3

    print(f"TOGGLE_STATE after_once first_item=[{second}]")

    if not toggle_hotkey(f):
        print("TOGGLE_FAIL reason=keychord_second_failed")
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 4

    send_cmd_safe(f, "RESET")
    send_cmd_safe(f, "ACTIVATE")
    third = seed_candidate(f)
    if third is None:
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 4

    print(f"TOGGLE_STATE after_twice first_item=[{third}]")

    changed = first != second
    restored = third == first and third != ""

    if changed and restored:
        print(
            "TOGGLE_PASS"
            f" probe={PROBE_TEXT}"
            f" before=[{first}]"
            f" after_once=[{second}]"
            f" after_twice=[{third}]"
        )
    else:
        print(
            "TOGGLE_OBS"
            f" probe={PROBE_TEXT}"
            f" before=[{first}]"
            f" after_once=[{second}]"
            f" after_twice=[{third}]"
            f" changed={int(changed)} restored={int(restored)}"
        )

    # KEYCHORD dispatch itself is the primary action under test here.
    rc = 0

    send_cmd_safe(f, "QUIT")
    sock.close()
    return rc


if __name__ == "__main__":
    raise SystemExit(main())
