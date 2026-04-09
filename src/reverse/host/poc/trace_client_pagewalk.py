import os
import re
import socket
import sys
import time

PORT = int(os.environ.get("POC_HOST_PORT", "22702"))
MAX_PAGE_STEPS = int(os.environ.get("POC_PAGE_STEPS", "64"))


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


def first_proc(trace_line):
    if not trace_line:
        return -1
    m = re.search(r"data=[^\{]*\{proc=(\d+)", trace_line)
    if not m:
        return -1
    return int(m.group(1))


def parse_primary_candidate(line):
    if not line:
        return None
    m = re.search(
        r"#0\{enc=([AW]) count=(\d+) sel=(\d+) pageStart=(\d+) pageSize=(\d+) items=\[(.*?)\]\}",
        line,
    )
    if not m:
        return None
    return {
        "enc": m.group(1),
        "count": int(m.group(2)),
        "sel": int(m.group(3)),
        "page_start": int(m.group(4)),
        "page_size": int(m.group(5)),
        "items": m.group(6),
        "raw": line,
    }


def candidate_signature(info):
    if not info:
        return ""
    return info["items"]


def signature_label(sig):
    if not sig:
        return "<empty>"
    first = sig.split("|", 1)[0].strip()
    if not first:
        first = "<blank>"
    return f"{first}:{len(sig)}"


def poll_candidate(f, attempts=8, delay_sec=0.10):
    best_info = None
    hit_try = -1
    for i in range(1, attempts + 1):
        line = send_cmd_safe(f, "CAND")
        if line is None:
            break

        info = parse_primary_candidate(line)
        if info and (best_info is None or info["count"] > best_info["count"]):
            best_info = info

        if info and info["count"] > 0:
            hit_try = i
            break

        time.sleep(delay_sec)

    return best_info, hit_try


def candidate_score(info):
    if not info:
        return (-1, -1, -1)
    is_multi = 1 if info["page_size"] > 0 and info["count"] > info["page_size"] else 0
    return (is_multi, info["count"], info["page_size"])


def walk_pages(f, start_info):
    first_sig = candidate_signature(start_info)
    unique_sigs = [first_sig]
    down_path = [first_sig]
    reached_end = False

    for _ in range(MAX_PAGE_STEPS):
        line = send_cmd_safe(f, "PAGEDOWN")
        if line is None:
            return None, None, False, False, "connection_lost_on_pagedown"

        info = parse_primary_candidate(line)
        if not info:
            return None, None, False, False, "pagedown_no_candidate"

        sig = candidate_signature(info)
        down_path.append(sig)

        if sig == unique_sigs[-1]:
            # No-op paging at last page.
            reached_end = True
            break

        if sig in unique_sigs:
            # Paging wrapped/cycled; boundary before this repeated signature is last page.
            reached_end = True
            break

        unique_sigs.append(sig)

    if not reached_end:
        return down_path, None, False, False, "end_page_not_reached"

    last_sig = unique_sigs[-1]
    current_sig = down_path[-1]

    # If we stopped on a repeated earlier page (for example wrapped to first),
    # move back to the discovered last page before walking upward.
    if current_sig != last_sig:
        synced = False
        for _ in range(4):
            line = send_cmd_safe(f, "PAGEUP")
            if line is None:
                return down_path, None, True, False, "connection_lost_sync_last"
            info = parse_primary_candidate(line)
            if not info:
                return down_path, None, True, False, "pageup_no_candidate_sync_last"
            current_sig = candidate_signature(info)
            if current_sig == last_sig:
                synced = True
                break
        if not synced:
            return down_path, None, True, False, "failed_to_sync_last_page"

    up_path = [current_sig]
    reached_start = current_sig == first_sig
    stall = 0
    seen_up = {current_sig}

    while not reached_start and len(up_path) <= MAX_PAGE_STEPS + 1:
        line = send_cmd_safe(f, "PAGEUP")
        if line is None:
            return down_path, up_path, True, False, "connection_lost_on_pageup"

        info = parse_primary_candidate(line)
        if not info:
            return down_path, up_path, True, False, "pageup_no_candidate"

        sig = candidate_signature(info)
        up_path.append(sig)

        if sig == first_sig:
            reached_start = True
            break

        if sig == current_sig:
            stall += 1
        else:
            current_sig = sig
            stall = 0

        if sig in seen_up:
            break
        seen_up.add(sig)

        if stall >= 2:
            break

    if not reached_start:
        return down_path, up_path, True, False, "start_page_not_reached"

    return down_path, up_path, True, True, "ok"


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
        print(f"PAGEWALK_FAIL reason=hello_failed detail={type(e).__name__}:{e}")
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

    trace = send_cmd_safe(f, "TRACEU ni")
    if trace is None:
        sock.close()
        return 1

    proc = first_proc(trace)
    if proc <= 0:
        print(f"PAGEWALK_FAIL reason=process_gate first_proc={proc}")
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 2

    best_seed = "TRACEU ni"
    best_info, best_try = poll_candidate(f, attempts=6, delay_sec=0.08)

    if not best_info or best_info["count"] <= 0:
        line = send_cmd_safe(f, "TRACEPIPEU ni")
        if line is not None:
            best_seed = "TRACEPIPEU ni"
            best_info, best_try = poll_candidate(f, attempts=6, delay_sec=0.08)

    if not best_info or best_info["count"] <= 0:
        print("PAGEWALK_FAIL reason=candidate_not_visible")
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 3

    print(
        "PAGEWALK_INFO"
        f" seed={best_seed}"
        f" cand_try={best_try}"
        f" enc={best_info['enc']}"
        f" count={best_info['count']}"
        f" page_size={best_info['page_size']}"
        f" start_page={best_info['page_start']}"
        f" first_item={signature_label(candidate_signature(best_info))}"
    )

    down_path, up_path, reached_end, reached_start, reason = walk_pages(f, best_info)

    if not reached_end:
        print(f"PAGEWALK_FAIL reason={reason}")
        if down_path is not None:
            print("PAGEWALK_DOWN_PATH", down_path)
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 4

    if not reached_start:
        print(f"PAGEWALK_FAIL reason={reason}")
        print("PAGEWALK_DOWN_PATH", down_path)
        if up_path is not None:
            print("PAGEWALK_UP_PATH", up_path)
        send_cmd_safe(f, "QUIT")
        sock.close()
        return 5

    print(
        "PAGEWALK_PASS"
        f" seed={best_seed}"
        f" start_page={best_info['page_start']}"
        f" unique_pages={max(1, len(set(down_path)))}"
        f" down_steps={len(down_path)-1}"
        f" up_steps={len(up_path)-1}"
    )
    print("PAGEWALK_DOWN_PATH", [signature_label(x) for x in down_path])
    print("PAGEWALK_UP_PATH", [signature_label(x) for x in up_path])

    send_cmd_safe(f, "QUIT")
    sock.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
