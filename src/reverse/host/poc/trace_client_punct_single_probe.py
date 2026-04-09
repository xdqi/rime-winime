import os
import re
import socket
import time

PORT = int(os.environ.get("POC_HOST_PORT", "22702"))
CASES_RAW = os.environ.get("POC_PUNCT_CASES", ",|.")


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


def cand_data(line):
    if not line:
        return ""
    m = re.search(r"data=(.*)$", line)
    if not m:
        return ""
    return m.group(1).strip()


def first_candidate(line):
    if not line:
        return ""
    m = re.search(r"#0\{[^\}]*items=\[(.*?)\]\}", line)
    if not m:
        return ""
    parts = [x.strip() for x in m.group(1).split("|") if x.strip() and x.strip() != "..."]
    if not parts:
        return ""
    return parts[0]


def codepoints(s):
    return ",".join(f"U+{ord(ch):04X}" for ch in s)


def main():
    cases = [x for x in CASES_RAW.split("|") if x]
    if not cases:
        print("PUNCT_FAIL reason=no_cases")
        return 1

    sock = None
    for _ in range(400):
        try:
            sock = socket.create_connection(("127.0.0.1", PORT), timeout=0.5)
            break
        except OSError:
            time.sleep(0.05)

    if sock is None:
        print(f"PUNCT_FAIL reason=connect_failed port={PORT}")
        return 1

    sock.settimeout(30.0)
    f = sock.makefile("rwb", buffering=0)

    try:
        print("HELLO", recv_line(f))
    except (OSError, TimeoutError) as e:
        print(f"PUNCT_FAIL reason=hello_failed detail={type(e).__name__}:{e}")
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

    templates = [
        "TRACEU {t}",
        "TRACEPIPEU {t}",
        "PIPEU {t}",
        "KEYTEXTU {t}",
        "KEYPIPEU {t}",
    ]

    for t in cases:
        print(f"PUNCT_CASE input=[{t}] cps=[{codepoints(t)}]")

        if send_cmd_safe(f, "RESET") is None:
            print("PUNCT_FAIL reason=reset_failed")
            send_cmd_safe(f, "QUIT")
            sock.close()
            return 2
        if send_cmd_safe(f, "ACTIVATE") is None:
            print("PUNCT_FAIL reason=activate_failed")
            send_cmd_safe(f, "QUIT")
            sock.close()
            return 2

        for tpl in templates:
            cmd = tpl.format(t=t)
            line = send_cmd_safe(f, cmd)
            if line is None:
                print("PUNCT_FAIL reason=seed_io_failed")
                send_cmd_safe(f, "QUIT")
                sock.close()
                return 3

            pre = send_cmd_safe(f, "PREEDIT")
            if pre is None:
                print("PUNCT_FAIL reason=preedit_failed")
                send_cmd_safe(f, "QUIT")
                sock.close()
                return 3

            cand = send_cmd_safe(f, "CAND")
            if cand is None:
                print("PUNCT_FAIL reason=cand_failed")
                send_cmd_safe(f, "QUIT")
                sock.close()
                return 3

            print(
                "PUNCT_OBS"
                f" input=[{t}]"
                f" cmd={cmd.split(' ')[0]}"
                f" comp=[{field_value(pre, 'comp')}]"
                f" read=[{field_value(pre, 'read')}]"
                f" result=[{field_value(pre, 'result')}]"
                f" cand_data=[{cand_data(cand)}]"
                f" first_cand=[{first_candidate(cand)}]"
            )

    send_cmd_safe(f, "QUIT")
    sock.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
