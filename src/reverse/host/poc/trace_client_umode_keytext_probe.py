import os
import re
import socket
import time

PORT = int(os.environ.get("POC_HOST_PORT", "22702"))
INPUT_TEXT = os.environ.get("POC_UMODE_TEXT", "uhspn")


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


def field_value(line, name):
    if not line:
        return ""
    m = re.search(rf"{name}=\[(.*?)\]", line)
    if not m:
        return ""
    return m.group(1).strip()


def parse_candidate_items(line):
    if not line:
        return []
    m = re.search(r"#0\{[^\}]*items=\[(.*?)\]\}", line)
    if not m:
        return []
    items = []
    for part in m.group(1).split("|"):
        token = part.strip()
        if token and token != "...":
            items.append(token)
    return items


def poll_cand(f, attempts=8, delay_sec=0.08):
    last = ""
    for _ in range(attempts):
        line = send_cmd_safe(f, "CAND")
        if line is None:
            return [], last
        last = line
        items = parse_candidate_items(line)
        if items:
            return items, line
        time.sleep(delay_sec)
    return [], last


def main():
    sock = None
    for _ in range(500):
        try:
            sock = socket.create_connection(("127.0.0.1", PORT), timeout=0.5)
            break
        except OSError:
            time.sleep(0.05)

    if sock is None:
        print(f"KEYUMODE_FAIL reason=connect_failed port={PORT}")
        return 1

    sock.settimeout(25.0)
    f = sock.makefile("rwb", buffering=0)

    try:
        print("HELLO", recv_line(f))
    except (OSError, TimeoutError) as e:
        print(f"KEYUMODE_FAIL reason=hello_failed detail={type(e).__name__}:{e}")
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

    seeds = [
        f"KEYTEXTU {INPUT_TEXT}",
        f"KEYPIPEU {INPUT_TEXT}",
    ]

    cand_items = []
    cand_line = ""
    used_seed = ""

    for cmd in seeds:
        line = send_cmd_safe(f, cmd)
        if line is None:
            print("KEYUMODE_FAIL reason=seed_io_failed")
            send_cmd_safe(f, "QUIT")
            sock.close()
            return 2

        pre = send_cmd_safe(f, "PREEDIT")
        if pre is None:
            print("KEYUMODE_FAIL reason=preedit_io_failed")
            send_cmd_safe(f, "QUIT")
            sock.close()
            return 2

        items = parse_candidate_items(line)
        if items:
            cand_items = items
            cand_line = line
            used_seed = cmd + " (direct)"
            break

        items, line2 = poll_cand(f)
        if items:
            cand_items = items
            cand_line = line2
            used_seed = cmd
            break

    if not cand_items:
        print(f"KEYUMODE_FAIL reason=no_candidates input={INPUT_TEXT}")
        if cand_line:
            print("KEYUMODE_CAND_LAST =>", cand_line)
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 3

    pick_line = send_cmd_safe(f, "PICK 1")
    pre_after = send_cmd_safe(f, "PREEDIT")
    cand_after = send_cmd_safe(f, "CAND")
    status_after = send_cmd_safe(f, "STATUS")

    print(
        "KEYUMODE_PASS"
        f" input={INPUT_TEXT}"
        f" seed={used_seed}"
        f" top_candidate={cand_items[0]}"
        f" candidate_count={len(cand_items)}"
        f" result_after_pick=[{field_value(pre_after, 'result')}]"
        f" comp_after_pick=[{field_value(pre_after, 'comp')}]"
        f" read_after_pick=[{field_value(pre_after, 'read')}]"
    )

    if pick_line:
        print("KEYUMODE_PICK_REPLY", pick_line)
    if cand_after:
        print("KEYUMODE_CAND_AFTER", cand_after)
    if status_after:
        print("KEYUMODE_STATUS_AFTER", status_after)

    send_cmd_safe(f, "QUIT")
    sock.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
