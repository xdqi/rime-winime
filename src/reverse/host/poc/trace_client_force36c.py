import os
import re
import socket
import sys
import time

PORT = int(os.environ.get("POC_HOST_PORT", "22702"))
ALLOW_ASCII_FALLBACK = os.environ.get("POC_ALLOW_ASCII_FALLBACK", "0") == "1"
RUN_HAN_TRACE_PROBE = os.environ.get("POC_HAN_TRACE_PROBE", "1") == "1"
VISUAL_KEY_INPUT = os.environ.get("POC_VISUAL_KEY_INPUT", "0") == "1"


def env_float(name, default):
    raw = os.environ.get(name)
    if raw is None:
        return float(default)
    try:
        return float(raw)
    except ValueError:
        return float(default)


VISUAL_STEP_DELAY_SEC = env_float("POC_VISUAL_STEP_DELAY_SEC", 0.0)
HOLD_BEFORE_QUIT_SEC = env_float("POC_HOLD_BEFORE_QUIT_SEC", 0.0)


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


def cmd_failed(line):
    return line.startswith("ERR")


def first_proc(trace_line):
    m = re.search(r"data=[^\{]*\{proc=(\d+)", trace_line)
    if not m:
        return -1
    return int(m.group(1))


def proc_from_key_ret(line):
    if not line:
        return -1
    m = re.search(r"\bprocess=(\d+)", line)
    if not m:
        return -1
    return int(m.group(1))


def nonempty_bracket_field(line, field):
    m = re.search(rf"{field}=\[(.*?)\]", line)
    if not m:
        return False
    return bool(m.group(1).strip())


def preedit_visible(line):
    return (
        nonempty_bracket_field(line, "comp")
        or nonempty_bracket_field(line, "read")
        or nonempty_bracket_field(line, "result")
    )


def trace_preedit_visible(trace_line):
    for token in re.findall(r"comp=\[(.*?)\]", trace_line):
        if token.strip():
            return True
    for token in re.findall(r"read=\[(.*?)\]", trace_line):
        if token.strip():
            return True
    return False


def candidate_count(line):
    counts = [int(x) for x in re.findall(r"\bcount=(\d+)", line)]
    if counts:
        return max(counts)
    return 0


def poll_candidate(f, attempts=8, delay_sec=0.10):
    best_count = 0
    best_line = ""
    hit_try = -1
    for i in range(1, attempts + 1):
        line = send_cmd_safe(f, "CAND")
        if line is None:
            break
        count = candidate_count(line)
        if count > best_count:
            best_count = count
            best_line = line
        if count > 0:
            hit_try = i
            break
        time.sleep(delay_sec)
    return best_count, best_line, hit_try


def run_visual_key_sequence(f, text):
    first_proc_seen = -1

    for ch in text:
        vk = ord(ch.upper())
        line = send_cmd_safe(f, f"KEY {vk:02X}")
        if line is None:
            return -1

        p = proc_from_key_ret(line)
        if first_proc_seen < 0 and p >= 0:
            first_proc_seen = p

        send_cmd_safe(f, "PREEDIT")
        send_cmd_safe(f, "CAND")

        if VISUAL_STEP_DELAY_SEC > 0:
            time.sleep(VISUAL_STEP_DELAY_SEC)

    return first_proc_seen


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
        print(f"POC_FAIL reason=hello_failed detail={type(e).__name__}:{e}")
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

    trace_cmd = "TRACEU nihao"
    trace = ""
    proc = -1

    if VISUAL_KEY_INPUT:
        trace_cmd = "KEYSEQ nihao (visual)"
        proc = run_visual_key_sequence(f, "nihao")
        if proc < 0:
            print("POC_FAIL reason=connection_lost_on_visual_keyseq")
            sock.close()
            return 6
    else:
        trace = send_cmd_safe(f, trace_cmd)
        if trace is None:
            print("POC_FAIL reason=connection_lost_on_unicode_trace")
            sock.close()
            return 6
        if cmd_failed(trace):
            print("POC_FAIL reason=unicode_trace_unavailable")
            send_cmd_safe(f, "QUIT")
            sock.close()
            return 5
        proc = first_proc(trace)

    preedit_probe = send_cmd_safe(f, "PREEDIT")
    if preedit_probe is None:
        print("POC_FAIL reason=connection_lost_on_preedit")
        sock.close()
        return 6

    candidate_max = 0
    candidate_hit_cmd = ""
    candidate_hit_try = -1
    candidate_line = ""

    # Capture a first candidate snapshot immediately after TRACE/PREEDIT.
    # In unstable sessions, later trigger sequences may hit transport errors.
    count, best_line, hit_try = poll_candidate(f, attempts=3, delay_sec=0.08)
    if count > candidate_max:
        candidate_max = count
        candidate_line = best_line
    if hit_try > 0:
        candidate_hit_cmd = f"{trace_cmd} (initial)"
        candidate_hit_try = hit_try

    trigger_cmds = [
        "TRACEPIPEU ni",
        "PIPEU ni",
        "TRACEPIPEU nihao",
        "PIPEU nihao",
    ]

    if ALLOW_ASCII_FALLBACK:
        trigger_cmds.extend([
            "TRACEPIPE ni",
            "PIPE ni",
            "TRACEPIPE nihao",
            "PIPE nihao",
        ])

    if candidate_max == 0:
        for cmd in trigger_cmds:
            line = send_cmd_safe(f, cmd)
            if line is None:
                break
            if cmd_failed(line):
                continue
            time.sleep(0.20)

            count, best_line, hit_try = poll_candidate(f, attempts=8, delay_sec=0.10)
            if count > candidate_max:
                candidate_max = count
                candidate_line = best_line
            if hit_try > 0:
                candidate_hit_cmd = cmd
                candidate_hit_try = hit_try
                break

    preedit_ok = trace_preedit_visible(trace) or preedit_visible(preedit_probe)
    cand_ok = candidate_max > 0

    # Optional Han-trace probe runs after main verdict inputs are collected,
    # so it cannot disturb candidate gating on the primary Unicode trace.
    if RUN_HAN_TRACE_PROBE:
        send_cmd_safe(f, "TRACEU 你好")

    if HOLD_BEFORE_QUIT_SEC > 0:
        print(f"HOLD => sleeping {HOLD_BEFORE_QUIT_SEC:.2f}s before QUIT")
        time.sleep(HOLD_BEFORE_QUIT_SEC)

    if proc <= 0:
        print(f"POC_FAIL first_proc={proc} reason=process_gate")
        rc = 2
    elif not preedit_ok:
        print("POC_FAIL reason=preedit_not_visible")
        rc = 3
    elif not cand_ok:
        print("POC_FAIL reason=candidate_not_visible")
        if candidate_line:
            print("POC_CAND_BEST =>", candidate_line)
        rc = 4
    else:
        print(
            "POC_PASS"
            f" first_proc={proc}"
            f" candidate_count={candidate_max}"
            f" trigger={candidate_hit_cmd}"
            f" cand_try={candidate_hit_try}"
        )
        rc = 0

    send_cmd_safe(f, "QUIT")
    sock.close()
    return rc


if __name__ == "__main__":
    raise SystemExit(main())
