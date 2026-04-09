import os
import re
import socket
import time

PORT = int(os.environ.get("POC_HOST_PORT", "22702"))
INPUT_TEXT = os.environ.get("POC_VMODE_TEXT", "v2012.1.1")
PICK_INDEX = int(os.environ.get("POC_VMODE_PICK", "1"))


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


def parse_process(line):
    if not line:
        return -1
    m = re.search(r"\bprocess=(\d+)", line)
    if not m:
        return -1
    return int(m.group(1))


def parse_items(line):
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


def parse_field(line, name):
    if not line:
        return ""
    m = re.search(rf"{name}=\[(.*?)\]", line)
    if not m:
        return ""
    return m.group(1).strip()


def key_candidates_for_char(ch):
    if "a" <= ch <= "z":
        return [ord(ch.upper())]
    if "A" <= ch <= "Z":
        return [ord(ch)]
    if "0" <= ch <= "9":
        return [ord(ch)]
    if ch == ".":
        # Prefer OEM period, then numpad decimal as fallback.
        return [0xBE, 0x6E]
    if ch == " ":
        return [0x20]
    if ch == ",":
        return [0xBC]
    if ch == "/":
        return [0xBF]
    if ch == "-":
        return [0xBD]
    if ch == "=":
        return [0xBB]
    return []


def press_text_as_keys(f, text):
    dot_vk = ""
    for idx, ch in enumerate(text, start=1):
        candidates = key_candidates_for_char(ch)
        if not candidates:
            print(f"VMODE_KEYSEQ_FAIL reason=unsupported_char index={idx} char=U+{ord(ch):04X}")
            return False, dot_vk

        chosen_vk = None
        chosen_proc = -1

        for vk in candidates:
            line = send_cmd_safe(f, f"KEY {vk:02X}")
            if line is None:
                print("VMODE_KEYSEQ_FAIL reason=key_send_failed")
                return False, dot_vk

            proc = parse_process(line)
            if chosen_vk is None:
                chosen_vk = vk
                chosen_proc = proc

            if proc > 0:
                chosen_vk = vk
                chosen_proc = proc
                break

        if ch == ".":
            dot_vk = f"0x{chosen_vk:02X}/proc={chosen_proc}"

        pre = send_cmd_safe(f, "PREEDIT")
        if pre is None:
            print("VMODE_KEYSEQ_FAIL reason=preedit_failed")
            return False, dot_vk

        if idx == len(text):
            cand_line = send_cmd_safe(f, "CAND")
            if cand_line is None:
                print("VMODE_KEYSEQ_FAIL reason=cand_failed")
                return False, dot_vk

    return True, dot_vk


def main():
    sock = None
    for _ in range(500):
        try:
            sock = socket.create_connection(("127.0.0.1", PORT), timeout=0.5)
            break
        except OSError:
            time.sleep(0.05)

    if sock is None:
        print(f"VMODE_KEYSEQ_FAIL reason=connect_failed port={PORT}")
        return 1

    sock.settimeout(25.0)
    f = sock.makefile("rwb", buffering=0)

    try:
        print("HELLO", recv_line(f))
    except (OSError, TimeoutError) as e:
        print(f"VMODE_KEYSEQ_FAIL reason=hello_failed detail={type(e).__name__}:{e}")
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

    ok, dot_vk = press_text_as_keys(f, INPUT_TEXT)
    if not ok:
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 2

    cand_line = ""
    cand_items = []
    for _ in range(10):
        line = send_cmd_safe(f, "CAND")
        if line is None:
            break
        items = parse_items(line)
        if items:
            cand_line = line
            cand_items = items
            break
        time.sleep(0.08)

    if not cand_items:
        print(f"VMODE_KEYSEQ_FAIL reason=no_candidates input={INPUT_TEXT} dot_vk={dot_vk}")
        if cand_line:
            print("VMODE_KEYSEQ_CAND_LAST", cand_line)
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 3

    pick_line = send_cmd_safe(f, f"PICK {PICK_INDEX}")
    pre_after = send_cmd_safe(f, "PREEDIT")
    cand_after = send_cmd_safe(f, "CAND")
    status_after = send_cmd_safe(f, "STATUS")

    print(
        "VMODE_KEYSEQ_PASS"
        f" input={INPUT_TEXT}"
        f" top_candidate={cand_items[0]}"
        f" candidate_count={len(cand_items)}"
        f" pick={PICK_INDEX}"
        f" dot_vk={dot_vk or 'n/a'}"
        f" result_after_pick=[{parse_field(pre_after, 'result')}]"
        f" comp_after_pick=[{parse_field(pre_after, 'comp')}]"
        f" read_after_pick=[{parse_field(pre_after, 'read')}]"
    )

    if pick_line:
        print("VMODE_KEYSEQ_PICK_REPLY", pick_line)
    if cand_after:
        print("VMODE_KEYSEQ_CAND_AFTER", cand_after)
    if status_after:
        print("VMODE_KEYSEQ_STATUS_AFTER", status_after)

    send_cmd_safe(f, "QUIT")
    sock.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
