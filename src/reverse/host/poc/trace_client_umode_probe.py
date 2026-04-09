import os
import re
import socket
import sys
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
    raw = m.group(1)
    items = []
    for part in raw.split("|"):
        token = part.strip()
        if not token or token == "...":
            continue
        items.append(token)
    return items


def first_proc(trace_line):
    if not trace_line:
        return -1
    m = re.search(r"data=[^\{]*\{proc=(\d+)", trace_line)
    if not m:
        return -1
    return int(m.group(1))


def main():
    sock = None
    for _ in range(500):
        try:
            sock = socket.create_connection(("127.0.0.1", PORT), timeout=0.5)
            break
        except OSError:
            time.sleep(0.05)

    if sock is None:
        print(f"UMODE_FAIL reason=connect_failed port={PORT}")
        return 1

    sock.settimeout(25.0)
    f = sock.makefile("rwb", buffering=0)

    try:
        print("HELLO", recv_line(f))
    except (OSError, TimeoutError) as e:
        print(f"UMODE_FAIL reason=hello_failed detail={type(e).__name__}:{e}")
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

    trace_cmd = f"TRACEU {INPUT_TEXT}"
    trace_line = send_cmd_safe(f, trace_cmd)
    if trace_line is None:
        print("UMODE_FAIL reason=trace_failed")
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 2

    proc = first_proc(trace_line)
    if proc <= 0:
        print(f"UMODE_FAIL reason=process_gate first_proc={proc}")
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 3

    pre_before = send_cmd_safe(f, "PREEDIT")
    cand_items = []
    cand_line = ""
    candidate_seed = trace_cmd

    for _ in range(1, 10):
        line = send_cmd_safe(f, "CAND")
        if line is None:
            break
        items = parse_candidate_items(line)
        if items:
            cand_items = items
            cand_line = line
            break
        time.sleep(0.08)

    if not cand_items:
        trigger_cmds = [
            f"TRACEPIPEU {INPUT_TEXT}",
            f"PIPEU {INPUT_TEXT}",
        ]
        for cmd in trigger_cmds:
            line = send_cmd_safe(f, cmd)
            if line is None:
                break

            # Some commands (notably PIPEU) directly return CAND_RET.
            direct_items = parse_candidate_items(line)
            if direct_items:
                cand_items = direct_items
                cand_line = line
                candidate_seed = cmd + " (direct)"
                break

            time.sleep(0.10)
            for _ in range(1, 7):
                pline = send_cmd_safe(f, "CAND")
                if pline is None:
                    break
                pitems = parse_candidate_items(pline)
                if pitems:
                    cand_items = pitems
                    cand_line = pline
                    candidate_seed = cmd
                    break
                time.sleep(0.08)

            if cand_items:
                break

    if not cand_items:
        print("UMODE_FAIL reason=no_candidates")
        if cand_line:
            print("UMODE_CAND_LAST =>", cand_line)
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 4

    pick_line = send_cmd_safe(f, "PICK 1")
    pre_after = send_cmd_safe(f, "PREEDIT")
    cand_after = send_cmd_safe(f, "CAND")
    status_after = send_cmd_safe(f, "STATUS")

    result_after = field_value(pre_after, "result")
    comp_after = field_value(pre_after, "comp")
    read_after = field_value(pre_after, "read")

    print(
        "UMODE_PASS"
        f" input={INPUT_TEXT}"
        f" seed={candidate_seed}"
        f" top_candidate={cand_items[0]}"
        f" candidate_count={len(cand_items)}"
        f" result_after_pick=[{result_after}]"
        f" comp_after_pick=[{comp_after}]"
        f" read_after_pick=[{read_after}]"
    )
    print("UMODE_CANDIDATES", cand_items)
    if pick_line:
        print("UMODE_PICK_REPLY", pick_line)
    if cand_after:
        print("UMODE_CAND_AFTER", cand_after)
    if status_after:
        print("UMODE_STATUS_AFTER", status_after)

    send_cmd_safe(f, "QUIT")
    sock.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
