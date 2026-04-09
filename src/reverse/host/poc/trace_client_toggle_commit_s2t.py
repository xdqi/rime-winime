import os
import re
import socket
import time


PORT = int(os.environ.get("POC_HOST_PORT", "22702"))
WARM_TEXT = os.environ.get("POC_TOGGLE_WARM_TEXT", "ni")
PROBE_TEXTS = [
    t.strip()
    for t in os.environ.get("POC_TOGGLE_PROBES", "zhongguo,hou,fa,long").split(",")
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


def field_value(line, key):
    if not line:
        return ""
    m = re.search(rf"{key}=\[(.*?)\]", line)
    if not m:
        return ""
    return m.group(1).strip()


def conv_first_item(line):
    if not line:
        return ""
    m = re.search(r"items=\[(.*?)\]", line)
    if not m:
        return ""
    items_raw = m.group(1).strip()
    if not items_raw:
        return ""
    first = items_raw.split("|", 1)[0].strip()
    if first == "...":
        return ""
    return first


def toggle_hotkey(f):
    line = send_cmd_safe(f, "KEYCHORD 46 1 1 0")
    return line is not None and not line.startswith("ERR")


def run_single_probe(f, state_name, probe_text):
    send_cmd_safe(f, "RESET")
    send_cmd_safe(f, "ACTIVATE")

    trace_line = send_cmd_safe(f, f"TRACEU {probe_text}")
    if trace_line is None:
        return None

    conv_line = send_cmd_safe(f, "CONV")
    if conv_line is None:
        return None

    conv_first = conv_first_item(conv_line)
    if not conv_first:
        conv_line_fallback = send_cmd_safe(f, f"CONV {probe_text}")
        if conv_line_fallback is None:
            return None
        conv_first = conv_first_item(conv_line_fallback)

    # Try to force a concrete commit and then capture result string.
    send_cmd_safe(f, "KEY 20")
    pre_before_commit = send_cmd_safe(f, "PREEDIT")
    if pre_before_commit is None:
        return None

    send_cmd_safe(f, "COMMIT")
    pre_after_commit = send_cmd_safe(f, "PREEDIT")
    if pre_after_commit is None:
        return None

    result_text = field_value(pre_after_commit, "result")
    if not result_text:
        result_text = field_value(pre_before_commit, "result")

    read_text = field_value(pre_after_commit, "read")
    if not read_text:
        read_text = field_value(pre_before_commit, "read")

    comp_text = field_value(pre_after_commit, "comp")
    if not comp_text:
        comp_text = field_value(pre_before_commit, "comp")

    signal = result_text or conv_first

    print(
        "S2T_SAMPLE"
        f" state={state_name}"
        f" probe={probe_text}"
        f" signal=[{signal}]"
        f" conv_first=[{conv_first}]"
        f" result=[{result_text}]"
        f" read=[{read_text}]"
        f" comp=[{comp_text}]"
    )

    return {
        "probe": probe_text,
        "signal": signal,
        "conv_first": conv_first,
        "result": result_text,
        "read": read_text,
        "comp": comp_text,
    }


def run_state(f, state_name):
    out = {}
    for probe in PROBE_TEXTS:
        sample = run_single_probe(f, state_name, probe)
        if sample is None:
            return None
        out[probe] = sample
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
        print(f"S2T_FAIL reason=hello_failed detail={type(e).__name__}:{e}")
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

    if send_cmd_safe(f, f"TRACEU {WARM_TEXT}") is None:
        sock.close()
        return 2

    before = run_state(f, "before")
    if before is None:
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 2

    if not toggle_hotkey(f):
        print("S2T_FAIL reason=keychord_unavailable")
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 3

    after_once = run_state(f, "after_once")
    if after_once is None:
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 3

    if not toggle_hotkey(f):
        print("S2T_FAIL reason=keychord_second_failed")
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 4

    after_twice = run_state(f, "after_twice")
    if after_twice is None:
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 4

    changed = []
    restored = []
    nonempty_pairs = []
    for probe in PROBE_TEXTS:
        b = before[probe]["signal"]
        a1 = after_once[probe]["signal"]
        a2 = after_twice[probe]["signal"]
        if b and a1:
            nonempty_pairs.append(probe)
            if b != a1:
                changed.append(probe)
                if a2 == b:
                    restored.append(probe)

    if changed and len(restored) == len(changed):
        print(
            "S2T_PASS"
            f" probes={','.join(PROBE_TEXTS)}"
            f" compared={','.join(nonempty_pairs)}"
            f" changed={','.join(changed)}"
            f" restored={','.join(restored)}"
        )
    else:
        print(
            "S2T_OBS"
            f" probes={','.join(PROBE_TEXTS)}"
            f" compared={','.join(nonempty_pairs)}"
            f" changed={','.join(changed)}"
            f" restored={','.join(restored)}"
        )

    send_cmd_safe(f, "QUIT")
    sock.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())