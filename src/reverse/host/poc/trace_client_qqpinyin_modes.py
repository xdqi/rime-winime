import os
import re
import socket
import time

PORT = int(os.environ.get("POC_HOST_PORT", "22912"))


def recv_line(f):
    return f.readline().decode("utf-8", "replace").rstrip("\r\n")


def send_cmd(f, cmd):
    f.write((cmd + "\n").encode())
    line = recv_line(f)
    print(f"{cmd} => {line}")
    return line


def first_proc(trace_line):
    m = re.search(r"data=[^\{]*\{proc=(\d+)", trace_line)
    if not m:
        return -1
    return int(m.group(1))


def candidate_count(line):
    nums = [int(x) for x in re.findall(r"\bcount=(\d+)", line)]
    if nums:
        return max(nums)
    return 0


def has_items(line):
    m = re.search(r"items=\[(.*?)\]", line)
    if not m:
        return False
    return bool(m.group(1).strip())


def has_preedit_content(line):
    for fld in ("comp", "read", "result"):
        m = re.search(rf"{fld}=\[(.*?)\]", line)
        if m and m.group(1).strip():
            return True
    return False


def extract_candidate_items(line):
    m = re.search(r"items=\[(.*?)\]", line)
    if not m:
        return []
    raw = m.group(1).strip()
    if not raw:
        return []
    return [x.strip() for x in raw.split("|") if x.strip()]


def extract_result_text(line):
    m = re.search(r"result=\[(.*?)\]", line)
    if not m:
        return ""
    return m.group(1).strip()


def normalize_surface_text(text):
    # Candidate items may include reading hints like 騳(dú); drop hints for compare.
    text = re.sub(r"\([^)]*\)", "", text)
    text = text.replace(" ", "").strip()
    return text


def check_commit_consistency(cand_before_line, preedit_line):
    items = extract_candidate_items(cand_before_line)
    result = extract_result_text(preedit_line)
    if not items or not result:
        return {
            "checkable": False,
            "result": result,
            "first": items[0] if items else "",
            "in_list": False,
            "first_match": False,
        }

    norm_result = normalize_surface_text(result)
    norm_items = [normalize_surface_text(x) for x in items]

    return {
        "checkable": True,
        "result": result,
        "first": items[0],
        "in_list": norm_result in norm_items,
        "first_match": norm_result == norm_items[0],
    }


def has_nihao_preedit(line):
    for fld in ("comp", "read"):
        m = re.search(rf"{fld}=\[(.*?)\]", line)
        if not m:
            continue
        # Some IMEs segment nihao as ni'hao; normalize punctuation before compare.
        normalized = re.sub(r"['\s]", "", m.group(1).lower())
        if normalized == "nihao":
            return True
    return False


def run_nihao(f):
    trace = send_cmd(f, "TRACEU nihao")
    preedit = send_cmd(f, "PREEDIT")
    cand = send_cmd(f, "CAND")

    proc_ok = first_proc(trace) > 0
    preedit_ok = has_nihao_preedit(trace) or has_nihao_preedit(preedit)
    cand_ok = candidate_count(cand) > 0 or has_items(cand)
    passed = proc_ok and preedit_ok and cand_ok

    if passed:
        print(
            "NIHAO_PASS"
            f" proc={first_proc(trace)}"
            f" cand_count={candidate_count(cand)}"
        )
    else:
        print(
            "NIHAO_FAIL"
            f" proc={first_proc(trace)}"
            f" preedit_ok={int(preedit_ok)}"
            f" cand_count={candidate_count(cand)}"
        )

    return passed


def run_mode_case(f, mode_name, text):
    trace = send_cmd(f, f"TRACEU {text}")
    cand_before = send_cmd(f, "CAND")
    pipe = send_cmd(f, f"TRACEPIPEU {text}")
    preedit = send_cmd(f, "PREEDIT")
    cand_after = send_cmd(f, "CAND")

    proc_ok = first_proc(trace) > 0
    cand_ok = (
        candidate_count(cand_before) > 0
        or has_items(cand_before)
        or candidate_count(cand_after) > 0
        or has_items(cand_after)
    )
    preedit_ok = has_preedit_content(preedit)
    commit = check_commit_consistency(cand_before, preedit)

    print(
        f"{mode_name}_CAND_BEFORE"
        f" input={text}"
        f" cand_count={candidate_count(cand_before)}"
        f" line={cand_before}"
    )
    print(
        f"{mode_name}_CAND_AFTER"
        f" input={text}"
        f" cand_count={candidate_count(cand_after)}"
        f" line={cand_after}"
    )

    print(
        f"{mode_name}_CASE"
        f" input={text}"
        f" proc={first_proc(trace)}"
        f" cand_before={candidate_count(cand_before)}"
        f" cand_after={candidate_count(cand_after)}"
        f" cand_ok={int(cand_ok)}"
        f" preedit_ok={int(preedit_ok)}"
    )
    print(
        f"{mode_name}_COMMIT_CHECK"
        f" input={text}"
        f" checkable={int(commit['checkable'])}"
        f" in_list={int(commit['in_list'])}"
        f" first_match={int(commit['first_match'])}"
        f" result=[{commit['result']}]"
        f" first=[{commit['first']}]"
    )
    if commit["checkable"] and not commit["in_list"]:
        print(
            f"{mode_name}_WARN"
            f" input={text}"
            f" reason=commit_not_in_candidates"
        )
    elif commit["checkable"] and not commit["first_match"]:
        print(
            f"{mode_name}_WARN"
            f" input={text}"
            f" reason=commit_not_first_candidate"
        )

    return {
        "trace": trace,
        "cand_before": cand_before,
        "pipe": pipe,
        "preedit": preedit,
        "cand_after": cand_after,
        "proc_ok": proc_ok,
        "cand_ok": cand_ok,
        "preedit_ok": preedit_ok,
        "commit_checkable": commit["checkable"],
        "commit_in_list": commit["in_list"],
        "commit_first_match": commit["first_match"],
    }


def summarize_mode(mode_name, results):
    proc_hits = sum(1 for r in results if r["proc_ok"])
    cand_hits = sum(1 for r in results if r["cand_ok"])
    preedit_hits = sum(1 for r in results if r["preedit_ok"])
    checkable_hits = sum(1 for r in results if r["commit_checkable"])
    in_list_hits = sum(1 for r in results if r["commit_in_list"])
    first_match_hits = sum(1 for r in results if r["commit_first_match"])

    passed = proc_hits > 0 and (cand_hits > 0 or preedit_hits > 0)
    if passed:
        print(
            f"{mode_name}_PASS"
            f" proc_hits={proc_hits}"
            f" cand_hits={cand_hits}"
            f" preedit_hits={preedit_hits}"
            f" commit_checkable={checkable_hits}"
            f" commit_in_list={in_list_hits}"
            f" commit_first_match={first_match_hits}"
        )
    else:
        print(
            f"{mode_name}_FAIL"
            f" proc_hits={proc_hits}"
            f" cand_hits={cand_hits}"
            f" preedit_hits={preedit_hits}"
            f" commit_checkable={checkable_hits}"
            f" commit_in_list={in_list_hits}"
            f" commit_first_match={first_match_hits}"
        )
    return passed


def connect_with_retry(port):
    sock = None
    for _ in range(400):
        try:
            sock = socket.create_connection(("127.0.0.1", port), timeout=0.5)
            break
        except OSError:
            time.sleep(0.05)
    return sock


def main():
    sock = connect_with_retry(PORT)
    if sock is None:
        print(f"connect failed port={PORT}")
        return 10

    sock.settimeout(30.0)
    f = sock.makefile("rwb", buffering=0)

    try:
        print("HELLO", recv_line(f))
        send_cmd(f, "STATUS")
        send_cmd(f, "ACTIVATE")
        send_cmd(f, "CP 936")

        nihao_pass = run_nihao(f)
        if not nihao_pass:
            send_cmd(f, "QUIT")
            sock.close()
            return 2

        # V mode examples from official help page:
        # - v + digits (number conversion)
        # - v + date-like input
        v_results = [
            run_mode_case(f, "VMODE", "v123"),
            run_mode_case(f, "VMODE", "v2012.1.1"),
            run_mode_case(f, "VMODE", "v1"),
        ]
        v_pass = summarize_mode("VMODE", v_results)

        # U mode examples from official help page:
        # - uhspn (stroke input)
        # - umama (component mode sample from docs)
        u_results = [
            run_mode_case(f, "UMODE", "uhspn"),
            run_mode_case(f, "UMODE", "umama"),
            run_mode_case(f, "UMODE", "u12345"),
        ]
        u_pass = summarize_mode("UMODE", u_results)

        send_cmd(f, "QUIT")
        sock.close()

        if v_pass and u_pass:
            print("QQPY_MODE_PASS")
            return 0
        if not v_pass and not u_pass:
            print("QQPY_MODE_FAIL v=0 u=0")
            return 5
        if not v_pass:
            print("QQPY_MODE_FAIL v=0 u=1")
            return 6
        print("QQPY_MODE_FAIL v=1 u=0")
        return 7
    except Exception as exc:
        print(f"QQPY_MODE_ERROR {type(exc).__name__}: {exc}")
        try:
            send_cmd(f, "QUIT")
        except Exception:
            pass
        sock.close()
        return 11


if __name__ == "__main__":
    raise SystemExit(main())
