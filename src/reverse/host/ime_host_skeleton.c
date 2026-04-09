#define WIN32_LEAN_AND_MEAN
#include <winsock2.h>
#include <ws2tcpip.h>
#include <windows.h>
#include <imm.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
#include <ctype.h>

#ifndef IACE_DEFAULT
#define IACE_DEFAULT 0x0010
#endif

#ifndef ISC_SHOWUIALL
#define ISC_SHOWUIALL 0xC000000FL
#endif

#ifndef IME_INVALID_HOTKEY
#define IME_INVALID_HOTKEY ((DWORD)-1)
#endif

#ifndef IME_PROP_KBD_CHAR_FIRST
#define IME_PROP_KBD_CHAR_FIRST 0x00000002
#endif

#ifndef IME_PROP_ACCEPT_WIDE_VKEY
#define IME_PROP_ACCEPT_WIDE_VKEY 0x00000020
#endif

#ifndef IPHK_PROCESSBYIME
#define IPHK_PROCESSBYIME 0x00000002
#endif

/* These APIs exist in IMM32 but may be absent from some mingw headers. */
WINUSERAPI DWORD WINAPI ImmProcessKey(HWND hWnd, HKL hKL, UINT vKey, LPARAM lKeyData, DWORD dwHotKeyID);
WINUSERAPI BOOL WINAPI ImmTranslateMessage(HWND hWnd, UINT message, WPARAM wParam, LPARAM lParam);

#define HOST_CLASS_NAME "SogouImeHostWindow"
#define DEFAULT_DLL_PATH "C:\\windows\\system32\\QQPinyin.ime"
#define DEFAULT_PORT 22345
#define RX_BUF_SIZE 8192

typedef struct _IMEINFO_COMPAT {
    DWORD dwPrivateDataSize;
    DWORD fdwProperty;
    DWORD fdwConversionCaps;
    DWORD fdwSentenceCaps;
    DWORD fdwUICaps;
    DWORD fdwSCSCaps;
    DWORD fdwSelectCaps;
} IMEINFO_COMPAT;

typedef struct _TRANSMSG_COMPAT {
    UINT message;
    WPARAM wParam;
    LPARAM lParam;
} TRANSMSG_COMPAT;

typedef struct _TRANSMSGLIST_COMPAT {
    UINT uMsgCount;
    TRANSMSG_COMPAT TransMsg[16];
} TRANSMSGLIST_COMPAT;

typedef BOOL (WINAPI *PFN_ImeInquire)(void *lpIMEInfo, LPWSTR lpszUIClass, DWORD dwSystemInfoFlags);
typedef BOOL (WINAPI *PFN_ImeSelect)(HIMC hIMC, BOOL fSelect);
typedef BOOL (WINAPI *PFN_ImeSetActiveContext)(HIMC hIMC, BOOL fActivate);
typedef UINT (WINAPI *PFN_ImeConversionList)(HIMC hIMC, LPCWSTR lpSource, void *lpDst, UINT dwBufLen, UINT uFlag);
typedef BOOL (WINAPI *PFN_ImeProcessKey)(HIMC hIMC, UINT vKey, LPARAM lKeyData, const BYTE *keyState);
typedef UINT (WINAPI *PFN_ImeToAsciiEx)(UINT vKey, UINT scanCode, const BYTE *keyState, void *transBuf, UINT state, HIMC hIMC);
typedef BOOL (WINAPI *PFN_NotifyIME)(HIMC hIMC, DWORD action, DWORD index, DWORD value);

typedef struct _HostState {
    HWND hwnd;
    HWND target_hwnd;
    HIMC himc;
    HMODULE ime;

    PFN_ImeInquire ime_inquire;
    PFN_ImeSelect ime_select;
    PFN_ImeSetActiveContext ime_set_active_context;
    PFN_ImeConversionList ime_conversion_list;
    PFN_ImeProcessKey ime_process_key;
    PFN_ImeToAsciiEx ime_to_ascii_ex;
    PFN_NotifyIME notify_ime;

    SOCKET listen_sock;
    SOCKET client_sock;

    char rx_buf[RX_BUF_SIZE];
    int rx_len;
    BOOL show_window;

    BOOL running;
    BOOL last_select;
    BOOL last_activate;
    BOOL last_process;
    DWORD last_imm_flags;
    UINT last_ascii;
    UINT last_msgs;
    BOOL last_notify;
    BOOL last_open;
    BOOL last_conv_set;
    DWORD last_conv_mode;
    DWORD last_sentence_mode;
    BOOL last_ctx_match;
    HKL last_hkl;
    BOOL last_hkl_switch;
    HIMC last_assoc_prev;
    HIMC last_ctx_now;
    DWORD last_cand_count;
    DWORD last_cand_sel;
    LONG last_comp_bytes;
    UINT cand_codepage;
    BOOL prefer_unicode_vkey;

    IMEINFO_COMPAT info;
    WCHAR ui_class[64];

    char last_error[256];
} HostState;

static HostState g_host;

static void pump_messages_once(void);
static int build_candidate_reply(HIMC himc, char *out, size_t out_size);
static int build_preedit_reply(HIMC himc, char *out, size_t out_size);

static HWND ime_target_hwnd(void) {
    return g_host.target_hwnd ? g_host.target_hwnd : g_host.hwnd;
}

static void set_last_errorf(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(g_host.last_error, sizeof(g_host.last_error), fmt, ap);
    va_end(ap);
}

static void appendf(char *dst, size_t dst_size, size_t *off, const char *fmt, ...) {
    va_list ap;
    int n;

    if (*off >= dst_size) {
        return;
    }

    va_start(ap, fmt);
    n = vsnprintf(dst + *off, dst_size - *off, fmt, ap);
    va_end(ap);

    if (n < 0) {
        return;
    }

    if ((size_t)n >= (dst_size - *off)) {
        *off = dst_size - 1;
    } else {
        *off += (size_t)n;
    }
}

static void sanitize_token(char *s) {
    int i;
    if (!s) {
        return;
    }
    for (i = 0; s[i]; ++i) {
        if (s[i] == '|' || s[i] == '\r' || s[i] == '\n' || s[i] == ';') {
            s[i] = ' ';
        }
    }
}

static int utf8_from_wide(const WCHAR *w, char *out, size_t out_size) {
    if (!w) {
        lstrcpynA(out, "<null>", (int)out_size);
        return 0;
    }
    if (!WideCharToMultiByte(CP_UTF8, 0, w, -1, out, (int)out_size, NULL, NULL)) {
        lstrcpynA(out, "<conv_err>", (int)out_size);
        return 0;
    }
    sanitize_token(out);
    return 1;
}

static int utf8_from_multibyte(const char *mb, UINT cp, char *out, size_t out_size) {
    WCHAR tmp[128];
    int n;

    if (!mb || !*mb) {
        lstrcpynA(out, "", (int)out_size);
        return 1;
    }

    n = MultiByteToWideChar(cp, 0, mb, -1, tmp, (int)(sizeof(tmp) / sizeof(tmp[0])));
    if (n <= 0 && cp != CP_ACP) {
        n = MultiByteToWideChar(CP_ACP, 0, mb, -1, tmp, (int)(sizeof(tmp) / sizeof(tmp[0])));
    }
    if (n <= 0) {
        lstrcpynA(out, "<decode_err>", (int)out_size);
        return 0;
    }

    return utf8_from_wide(tmp, out, out_size);
}

static int wide_prefers_packed_decode(const WCHAR *w) {
    int i;
    int suspicious = 0;

    if (!w || !*w) {
        return 0;
    }

    for (i = 0; w[i] && i < 8; ++i) {
        if (w[i] == 0xFFFD || (w[i] >= 0xE000 && w[i] <= 0xF8FF)) {
            ++suspicious;
        }
    }
    return suspicious > 0;
}

static int utf8_from_packed_wide_mb(const WCHAR *w, UINT cp, char *out, size_t out_size) {
    char mb[256];
    int m = 0;
    int i;

    if (!w) {
        lstrcpynA(out, "<null>", (int)out_size);
        return 0;
    }

    for (i = 0; w[i] && i < 120 && m < (int)sizeof(mb) - 2; ++i) {
        WCHAR wc = w[i];
        BYTE lo = (BYTE)(wc & 0xff);
        BYTE hi = (BYTE)((wc >> 8) & 0xff);
        mb[m++] = (char)lo;
        if (hi) {
            mb[m++] = (char)hi;
        }
    }
    mb[m] = 0;

    if (!m) {
        lstrcpynA(out, "", (int)out_size);
        return 1;
    }

    return utf8_from_multibyte(mb, cp, out, out_size);
}

static int looks_lossy_token(const char *s) {
    int i;
    int non_space = 0;
    int question = 0;

    if (!s || !*s) {
        return 1;
    }

    for (i = 0; s[i]; ++i) {
        unsigned char c = (unsigned char)s[i];
        if (c >= 0x80) {
            return 0;
        }
        if (c == ' ' || c == '.' || c == '\t') {
            continue;
        }
        ++non_space;
        if (c == '?') {
            ++question;
        }
    }

    if (!non_space) {
        return 1;
    }
    return question == non_space;
}

static int wide_looks_packed_ascii(const WCHAR *w) {
    int i;
    int pairs = 0;
    int printable_pairs = 0;

    if (!w || !*w) {
        return 0;
    }

    for (i = 0; w[i] && i < 12; ++i) {
        BYTE lo = (BYTE)(w[i] & 0xff);
        BYTE hi = (BYTE)((w[i] >> 8) & 0xff);
        if (!hi) {
            continue;
        }
        ++pairs;
        if (isprint(lo) && isprint(hi)) {
            ++printable_pairs;
        }
    }

    if (!pairs) {
        return 0;
    }
    return printable_pairs * 2 >= pairs;
}

static LONG read_comp_wide(HIMC himc, DWORD index, WCHAR *buf, size_t cch) {
    LONG bytes = ImmGetCompositionStringW(himc, index, NULL, 0);
    LONG copy;
    LONG max_bytes;

    if (!buf || cch == 0) {
        return bytes;
    }

    buf[0] = 0;
    if (bytes <= 0) {
        return bytes;
    }

    max_bytes = (LONG)((cch - 1) * sizeof(WCHAR));
    copy = bytes;
    if (copy > max_bytes) {
        copy = max_bytes;
    }

    if (ImmGetCompositionStringW(himc, index, buf, copy) < 0) {
        buf[0] = 0;
        return bytes;
    }

    buf[copy / (LONG)sizeof(WCHAR)] = 0;
    return bytes;
}

static void decode_ime_wide_text(const WCHAR *w, char *out, size_t out_size) {
    char packed[192];

    if (!w || !*w) {
        lstrcpynA(out, "", (int)out_size);
        return;
    }

    if (!utf8_from_wide(w, out, out_size)) {
        out[0] = 0;
    }

    if (!out[0] || looks_lossy_token(out) || wide_prefers_packed_decode(w) || wide_looks_packed_ascii(w)) {
        if (utf8_from_packed_wide_mb(w, g_host.cand_codepage, packed, sizeof(packed)) && !looks_lossy_token(packed)) {
            lstrcpynA(out, packed, (int)out_size);
            return;
        }
    }
}

static int build_preedit_reply(HIMC himc, char *out, size_t out_size) {
    WCHAR comp_w[192];
    WCHAR read_w[192];
    WCHAR result_w[192];
    char comp_utf8[192];
    char read_utf8[192];
    char result_utf8[192];
    LONG comp_bytes;
    LONG read_bytes;
    LONG result_bytes;
    LONG cursor;
    LONG delta;

    if (!himc) {
        snprintf(out, out_size, "PREEDIT_RET err=no_himc\n");
        return 0;
    }

    comp_bytes = read_comp_wide(himc, GCS_COMPSTR, comp_w, sizeof(comp_w) / sizeof(comp_w[0]));
    read_bytes = read_comp_wide(himc, GCS_COMPREADSTR, read_w, sizeof(read_w) / sizeof(read_w[0]));
    result_bytes = read_comp_wide(himc, GCS_RESULTSTR, result_w, sizeof(result_w) / sizeof(result_w[0]));

    decode_ime_wide_text(comp_w, comp_utf8, sizeof(comp_utf8));
    decode_ime_wide_text(read_w, read_utf8, sizeof(read_utf8));
    decode_ime_wide_text(result_w, result_utf8, sizeof(result_utf8));

    cursor = ImmGetCompositionStringW(himc, GCS_CURSORPOS, NULL, 0);
    delta = ImmGetCompositionStringW(himc, GCS_DELTASTART, NULL, 0);
    if (cursor < 0) {
        cursor = -1;
    }
    if (delta < 0) {
        delta = -1;
    }

    snprintf(
        out,
        out_size,
        "PREEDIT_RET compBytes=%ld readBytes=%ld resultBytes=%ld cursor=%ld delta=%ld comp=[%s] read=[%s] result=[%s]\n",
        comp_bytes > 0 ? comp_bytes : 0,
        read_bytes > 0 ? read_bytes : 0,
        result_bytes > 0 ? result_bytes : 0,
        cursor,
        delta,
        comp_utf8,
        read_utf8,
        result_utf8);

    return 0;
}

static int send_text(SOCKET s, const char *text) {
    size_t left = strlen(text);
    const char *p = text;
    while (left > 0) {
        int n = send(s, p, (int)left, 0);
        if (n <= 0) {
            return -1;
        }
        left -= (size_t)n;
        p += n;
    }
    return 0;
}

static UINT make_ime_vkey(UINT vk, UINT scan, BYTE *key_state, HKL hkl) {
    UINT ime_vk = vk;
    WCHAR wc = 0;
    int n;
    int use_unicode_pref = g_host.prefer_unicode_vkey || (g_host.info.fdwProperty & IME_PROP_UNICODE);

    if (!(g_host.info.fdwProperty & IME_PROP_KBD_CHAR_FIRST)) {
        return ime_vk;
    }

    if (use_unicode_pref) {
        /* Prefer Unicode packing for UTF-8 command paths and Unicode IMEs. */
        n = ToUnicode(vk, scan, key_state, &wc, 1, 0);
        if (n == 1) {
            ime_vk = (ime_vk & 0x00ff) | ((UINT)wc << 16);
            return ime_vk;
        }
        if (g_host.info.fdwProperty & IME_PROP_UNICODE) {
            return ime_vk;
        }
    }

    {
        WORD w = 0;
        n = ToAsciiEx(vk, scan, key_state, &w, 0, hkl);
        if (n > 0) {
            ime_vk = (ime_vk & 0x00ff) | ((UINT)w << 8);
            if ((BYTE)ime_vk == VK_PACKET) {
                if (!(g_host.info.fdwProperty & IME_PROP_ACCEPT_WIDE_VKEY)) {
                    ime_vk &= 0xffff;
                }
            } else {
                ime_vk &= 0xffff;
            }
        }
    }

    return ime_vk;
}

static HIMC get_active_himc(BOOL *borrowed) {
    HIMC ctx = ImmGetContext(ime_target_hwnd());
    if (ctx) {
        g_host.last_ctx_now = ctx;
        g_host.last_ctx_match = (ctx == g_host.himc);
        if (borrowed) {
            *borrowed = TRUE;
        }
        return ctx;
    }
    if (borrowed) {
        *borrowed = FALSE;
    }
    return g_host.himc;
}

static void release_active_himc(HIMC himc, BOOL borrowed) {
    if (borrowed && himc) {
        ImmReleaseContext(ime_target_hwnd(), himc);
    }
}

static int reply_with_candidate_snapshot(char *out, size_t out_size) {
    BOOL borrowed = FALSE;
    HIMC himc = get_active_himc(&borrowed);
    build_candidate_reply(himc, out, out_size);
    release_active_himc(himc, borrowed);
    return 0;
}

static int build_candidate_reply(HIMC himc, char *out, size_t out_size) {
    DWORD list_count = 0;
    DWORD total_size = 0;
    DWORD list_probe_limit = 0;
    LONG read_bytes = 0;
    WCHAR comp_w[192];
    WCHAR read_w[192];
    char comp_utf8[192];
    char read_utf8[192];
    size_t off = 0;
    DWORD idx;
    DWORD collected = 0;
    DWORD sel = 0;
    int any_list = 0;

    if (!himc) {
        snprintf(out, out_size, "CAND_RET err=no_himc\n");
        return 0;
    }

    g_host.last_comp_bytes = read_comp_wide(himc, GCS_COMPSTR, comp_w, sizeof(comp_w) / sizeof(comp_w[0]));
    read_bytes = read_comp_wide(himc, GCS_COMPREADSTR, read_w, sizeof(read_w) / sizeof(read_w[0]));
    decode_ime_wide_text(comp_w, comp_utf8, sizeof(comp_utf8));
    decode_ime_wide_text(read_w, read_utf8, sizeof(read_utf8));

    total_size = ImmGetCandidateListCountW(himc, &list_count);
    if (!list_count) {
        total_size = ImmGetCandidateListCountA(himc, &list_count);
    }

    list_probe_limit = list_count ? list_count : 4;

    if (!list_count && !total_size) {
        g_host.last_cand_count = 0;
        g_host.last_cand_sel = 0;
        snprintf(
            out,
            out_size,
            "CAND_RET count=0 cp=%u compBytes=%ld readBytes=%ld lists=%lu totalBytes=%lu comp=[%s] read=[%s]\n",
            (unsigned)g_host.cand_codepage,
            (long)g_host.last_comp_bytes,
            read_bytes > 0 ? read_bytes : 0,
            (unsigned long)list_count,
            (unsigned long)total_size,
            comp_utf8,
            read_utf8);
        return 0;
    }

    appendf(
        out,
        out_size,
        &off,
        "CAND_RET cp=%u lists=%lu totalBytes=%lu compBytes=%ld readBytes=%ld comp=[%s] read=[%s] data=",
        (unsigned)g_host.cand_codepage,
        (unsigned long)list_count,
        (unsigned long)total_size,
        (long)g_host.last_comp_bytes,
        read_bytes > 0 ? read_bytes : 0,
        comp_utf8,
        read_utf8);

    for (idx = 0; idx < list_probe_limit && idx < 4; ++idx) {
        DWORD bytes = 0;
        DWORD bytes_w = 0;
        int is_ansi = 0;
        CANDIDATELIST *cand;
        CANDIDATELIST *cand_w = NULL;
        DWORD got;
        DWORD got_w = 0;
        DWORD i;

        bytes = ImmGetCandidateListW(himc, idx, NULL, 0);
        if (bytes) {
            is_ansi = 0;
        } else {
            bytes = ImmGetCandidateListA(himc, idx, NULL, 0);
            is_ansi = 1;
        }

        if (!bytes) {
            continue;
        }

        cand = (CANDIDATELIST *)malloc(bytes);
        if (!cand) {
            continue;
        }

        got = is_ansi ? ImmGetCandidateListA(himc, idx, cand, bytes)
                  : ImmGetCandidateListW(himc, idx, cand, bytes);
        if (!got || got > bytes) {
            free(cand);
            continue;
        }

        if (is_ansi) {
            bytes_w = ImmGetCandidateListW(himc, idx, NULL, 0);
            if (bytes_w) {
                cand_w = (CANDIDATELIST *)malloc(bytes_w);
                if (cand_w) {
                    got_w = ImmGetCandidateListW(himc, idx, cand_w, bytes_w);
                    if (!got_w || got_w > bytes_w) {
                        free(cand_w);
                        cand_w = NULL;
                        got_w = 0;
                    }
                }
            }
        }

        /* Many IMEs expose large internal aux lists (dwPageSize=0) that are
         * not user-facing candidates; skip them for stable host telemetry. */
        if (idx > 0 && cand->dwPageSize == 0 && cand->dwCount >= 64) {
            if (cand_w) {
                free(cand_w);
            }
            free(cand);
            continue;
        }

        if (!any_list) {
            any_list = 1;
        } else {
            appendf(out, out_size, &off, ";");
        }

        appendf(
            out,
            out_size,
            &off,
            "#%lu{enc=%c count=%lu sel=%lu pageStart=%lu pageSize=%lu items=[",
            (unsigned long)idx,
            is_ansi ? 'A' : 'W',
            (unsigned long)cand->dwCount,
            (unsigned long)cand->dwSelection,
            (unsigned long)cand->dwPageStart,
            (unsigned long)cand->dwPageSize);

        if (!collected) {
            sel = cand->dwSelection;
        }
        collected += cand->dwCount;

        for (i = 0; i < cand->dwCount && i < 8; ++i) {
            int need;
            char utf8[192];

            if (cand->dwOffset[i] >= got) {
                continue;
            }
            need = 0;
            if (is_ansi) {
                const char *mb = (const char *)((const BYTE *)cand + cand->dwOffset[i]);
                need = utf8_from_multibyte(mb, g_host.cand_codepage, utf8, sizeof(utf8));
                if (need && looks_lossy_token(utf8) && cand_w && i < cand_w->dwCount && cand_w->dwOffset[i] < got_w) {
                    const WCHAR *ww = (const WCHAR *)((const BYTE *)cand_w + cand_w->dwOffset[i]);
                    char fallback[192];
                    if (wide_prefers_packed_decode(ww) &&
                        utf8_from_packed_wide_mb(ww, g_host.cand_codepage, fallback, sizeof(fallback)) &&
                        !looks_lossy_token(fallback)) {
                        lstrcpynA(utf8, fallback, (int)sizeof(utf8));
                    } else if (utf8_from_wide(ww, fallback, sizeof(fallback)) && !looks_lossy_token(fallback)) {
                        lstrcpynA(utf8, fallback, (int)sizeof(utf8));
                    }
                }
            } else {
                const WCHAR *w = (const WCHAR *)((const BYTE *)cand + cand->dwOffset[i]);
                need = utf8_from_wide(w, utf8, sizeof(utf8));
                if (need && wide_prefers_packed_decode(w)) {
                    char packed[192];
                    if (utf8_from_packed_wide_mb(w, g_host.cand_codepage, packed, sizeof(packed)) && !looks_lossy_token(packed)) {
                        lstrcpynA(utf8, packed, (int)sizeof(utf8));
                    }
                }
            }

            if (!need) {
                lstrcpynA(utf8, "<decode_err>", (int)sizeof(utf8));
            }

            if (i > 0) {
                appendf(out, out_size, &off, "|");
            }
            appendf(out, out_size, &off, "%s", utf8);
        }
        if (cand->dwCount > 8) {
            appendf(out, out_size, &off, "|...");
        }
        appendf(out, out_size, &off, "]}");

        if (cand_w) {
            free(cand_w);
        }
        free(cand);
    }

    if (!any_list) {
        appendf(out, out_size, &off, "none");
    }
    appendf(out, out_size, &off, "\n");

    g_host.last_cand_count = collected;
    g_host.last_cand_sel = sel;

    return 0;
}

typedef struct _KeyChord {
    unsigned vk;
    BOOL ctrl;
    BOOL shift;
    BOOL alt;
} KeyChord;

static HKL get_target_hkl(void) {
    DWORD proc_id = 0;
    DWORD thread_id = GetWindowThreadProcessId(ime_target_hwnd(), &proc_id);
    HKL hkl = GetKeyboardLayout(thread_id);
    if (!hkl) {
        hkl = GetKeyboardLayout(0);
    }
    return hkl;
}

static int key_chord_from_wide_char(WCHAR wc, HKL hkl, KeyChord *chord) {
    SHORT packed;
    unsigned shift_state;

    if (!chord || wc < 0x20) {
        return 0;
    }

    packed = VkKeyScanExW(wc, hkl);
    if (packed == -1) {
        return 0;
    }

    chord->vk = (unsigned)(packed & 0xff);
    if (chord->vk == 0 || chord->vk == 0xff) {
        return 0;
    }

    shift_state = (unsigned)((packed >> 8) & 0xff);
    chord->shift = (shift_state & 0x01u) != 0;
    chord->ctrl = (shift_state & 0x02u) != 0;
    chord->alt = (shift_state & 0x04u) != 0;
    return 1;
}

static int key_chord_from_ascii_char(char c, HKL hkl, KeyChord *chord) {
    return key_chord_from_wide_char((WCHAR)(unsigned char)c, hkl, chord);
}

static int utf8_to_wide_text(const char *text, WCHAR *out, size_t out_cch) {
    int n;

    if (!out || out_cch == 0) {
        return -1;
    }

    out[0] = 0;
    if (!text) {
        return 0;
    }

    n = MultiByteToWideChar(CP_UTF8, 0, text, -1, out, (int)out_cch);
    if (n <= 0) {
        return -1;
    }

    return n - 1;
}

static void key_label_from_wide_char(WCHAR wc, char *out, size_t out_size) {
    WCHAR tmp[2];
    char utf8[32];

    if (wc == L' ') {
        lstrcpynA(out, "SP", (int)out_size);
        return;
    }

    if (wc < 0x20 || wc == L';' || wc == L'[' || wc == L']') {
        snprintf(out, out_size, "U+%04X", (unsigned)wc);
        return;
    }

    tmp[0] = wc;
    tmp[1] = 0;
    if (utf8_from_wide(tmp, utf8, sizeof(utf8)) && utf8[0]) {
        lstrcpynA(out, utf8, (int)out_size);
        return;
    }

    snprintf(out, out_size, "U+%04X", (unsigned)wc);
}

static void close_client(void) {
    if (g_host.client_sock != INVALID_SOCKET) {
        closesocket(g_host.client_sock);
        g_host.client_sock = INVALID_SOCKET;
    }
    g_host.rx_len = 0;
}

static LRESULT CALLBACK host_wnd_proc(HWND hwnd, UINT msg, WPARAM wparam, LPARAM lparam) {
    (void)wparam;
    (void)lparam;

    switch (msg) {
    case WM_DESTROY:
        PostQuitMessage(0);
        return 0;
    default:
        return DefWindowProcA(hwnd, msg, wparam, lparam);
    }
}

static int create_hidden_window(void) {
    WNDCLASSA wc;
    ATOM atom;
    DWORD ex_style;
    DWORD style;
    int x;
    int y;
    int w;
    int h;

    ZeroMemory(&wc, sizeof(wc));
    wc.lpfnWndProc = host_wnd_proc;
    wc.hInstance = GetModuleHandleA(NULL);
    wc.lpszClassName = HOST_CLASS_NAME;

    atom = RegisterClassA(&wc);
    if (!atom && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) {
        set_last_errorf("RegisterClassA failed: %lu", (unsigned long)GetLastError());
        return -1;
    }

    if (g_host.show_window) {
        ex_style = 0;
        style = WS_OVERLAPPEDWINDOW;
        x = CW_USEDEFAULT;
        y = CW_USEDEFAULT;
        w = 320;
        h = 240;
    } else {
        ex_style = WS_EX_TOOLWINDOW;
        style = WS_POPUP;
        x = -32000;
        y = -32000;
        w = 8;
        h = 8;
    }

    g_host.hwnd = CreateWindowExA(
        ex_style,
        HOST_CLASS_NAME,
        "Sogou IME Host",
        style,
        x,
        y,
        w,
        h,
        NULL,
        NULL,
        GetModuleHandleA(NULL),
        NULL);

    if (!g_host.hwnd) {
        set_last_errorf("CreateWindowExA failed: %lu", (unsigned long)GetLastError());
        return -1;
    }

    g_host.target_hwnd = CreateWindowExA(
        0,
        "EDIT",
        "",
        WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL | WS_TABSTOP,
        0,
        0,
        8,
        8,
        g_host.hwnd,
        NULL,
        GetModuleHandleA(NULL),
        NULL);

    if (!g_host.target_hwnd) {
        g_host.target_hwnd = g_host.hwnd;
    }

    return 0;
}

static int bind_exports(void) {
    g_host.ime_inquire = (PFN_ImeInquire)GetProcAddress(g_host.ime, "ImeInquire");
    g_host.ime_select = (PFN_ImeSelect)GetProcAddress(g_host.ime, "ImeSelect");
    g_host.ime_set_active_context = (PFN_ImeSetActiveContext)GetProcAddress(g_host.ime, "ImeSetActiveContext");
    g_host.ime_conversion_list = (PFN_ImeConversionList)GetProcAddress(g_host.ime, "ImeConversionList");
    g_host.ime_process_key = (PFN_ImeProcessKey)GetProcAddress(g_host.ime, "ImeProcessKey");
    g_host.ime_to_ascii_ex = (PFN_ImeToAsciiEx)GetProcAddress(g_host.ime, "ImeToAsciiEx");
    g_host.notify_ime = (PFN_NotifyIME)GetProcAddress(g_host.ime, "NotifyIME");

    if (!g_host.ime_inquire || !g_host.ime_select || !g_host.ime_process_key || !g_host.ime_to_ascii_ex) {
        set_last_errorf("Missing required exports");
        return -1;
    }

    return 0;
}

static void update_context_snapshot(void) {
    HWND target = ime_target_hwnd();
    HIMC ctx = ImmGetContext(target);
    g_host.last_ctx_now = ctx;
    g_host.last_ctx_match = (ctx == g_host.himc);
    if (ctx) {
        ImmReleaseContext(target, ctx);
    }
}

static void ensure_ime_layout(HWND target) {
    static const char *const kls[] = {
        "E0200804",
        "E0220804",
        "E0010804",
        "00000804",
        "E0200409",
        NULL
    };
    HKL current = GetKeyboardLayout(0);
    HKL hkl = NULL;
    int i;

    for (i = 0; kls[i]; ++i) {
        hkl = LoadKeyboardLayoutA(kls[i], KLF_ACTIVATE | KLF_SUBSTITUTE_OK | KLF_REORDER);
        if (hkl) {
            break;
        }
    }
    if (!hkl) {
        hkl = current;
    }

    g_host.last_hkl = hkl;
    g_host.last_hkl_switch = (hkl && hkl != current);

    if (hkl) {
        ActivateKeyboardLayout(hkl, 0);
        SendMessageA(target, WM_INPUTLANGCHANGEREQUEST, 0, (LPARAM)hkl);
        SendMessageA(target, WM_INPUTLANGCHANGE, 0, (LPARAM)hkl);
    }
}

static int activate_ime_path(void) {
    HIMC ctx;
    HIMC use_himc;
    HWND def_ime_wnd;
    HWND target = ime_target_hwnd();

    update_context_snapshot();
    ctx = g_host.last_ctx_now;
    use_himc = ctx ? ctx : g_host.himc;

    ensure_ime_layout(target);

    g_host.last_select = g_host.ime_select(use_himc, TRUE);
    g_host.last_open = ImmSetOpenStatus(use_himc, TRUE);
    g_host.last_conv_set = ImmSetConversionStatus(use_himc, IME_CMODE_NATIVE, 0);
    ImmGetConversionStatus(use_himc, &g_host.last_conv_mode, &g_host.last_sentence_mode);

    if (g_host.ime_set_active_context) {
        g_host.last_activate = g_host.ime_set_active_context(use_himc, TRUE);
    } else {
        g_host.last_activate = 0;
    }

    SendMessageA(target, WM_IME_SETCONTEXT, TRUE, ISC_SHOWUIALL);
    def_ime_wnd = ImmGetDefaultIMEWnd(target);
    if (def_ime_wnd && def_ime_wnd != g_host.hwnd) {
        SendMessageA(def_ime_wnd, WM_IME_SETCONTEXT, TRUE, ISC_SHOWUIALL);
        SendMessageA(def_ime_wnd, WM_IME_SELECT, TRUE, (LPARAM)use_himc);
        SendMessageA(def_ime_wnd, WM_IME_NOTIFY, IMN_SETOPENSTATUS, 0);
        SendMessageA(def_ime_wnd, WM_IME_NOTIFY, IMN_SETCONVERSIONMODE, 0);
    }

    update_context_snapshot();
    pump_messages_once();
    return 0;
}

static int init_context_and_select(void) {
    HWND target = ime_target_hwnd();

    ZeroMemory(&g_host.info, sizeof(g_host.info));
    ZeroMemory(g_host.ui_class, sizeof(g_host.ui_class));

    if (!g_host.himc) {
        g_host.himc = ImmCreateContext();
        if (!g_host.himc) {
            set_last_errorf("ImmCreateContext failed");
            return -1;
        }
    }

    g_host.last_assoc_prev = ImmAssociateContext(target, g_host.himc);
    ImmAssociateContext(g_host.hwnd, g_host.himc);
    ImmAssociateContextEx(target, g_host.himc, 0);
    ImmAssociateContextEx(g_host.hwnd, g_host.himc, 0);

    if (g_host.show_window) {
        ShowWindow(g_host.hwnd, SW_SHOW);
        UpdateWindow(g_host.hwnd);
        SetForegroundWindow(g_host.hwnd);
        SetFocus(target);
    } else {
        ShowWindow(g_host.hwnd, SW_SHOW);
        SetWindowPos(g_host.hwnd, HWND_TOPMOST, -32000, -32000, 8, 8, SWP_SHOWWINDOW);
        SetForegroundWindow(g_host.hwnd);
        SetFocus(target);
    }

    g_host.ime_inquire(&g_host.info, g_host.ui_class, 0);
    activate_ime_path();

    return 0;
}

static int init_ime(const char *dll_path) {
    g_host.ime = LoadLibraryA(dll_path);
    if (!g_host.ime) {
        set_last_errorf("LoadLibraryA failed: %lu", (unsigned long)GetLastError());
        return -1;
    }

    if (bind_exports() != 0) {
        return -1;
    }

    if (create_hidden_window() != 0) {
        return -1;
    }

    if (init_context_and_select() != 0) {
        return -1;
    }

    return 0;
}

static int reset_context(void) {
    if (g_host.himc) {
        g_host.ime_select(g_host.himc, FALSE);
        ImmAssociateContextEx(g_host.hwnd, NULL, IACE_DEFAULT);
        ImmDestroyContext(g_host.himc);
        g_host.himc = NULL;
    }

    return init_context_and_select();
}

static void pump_messages_once(void) {
    MSG msg;
    while (PeekMessageA(&msg, NULL, 0, 0, PM_REMOVE)) {
        TranslateMessage(&msg);
        DispatchMessageA(&msg);
    }
}

static LPARAM key_lparam_from_vk(UINT vk, BOOL key_up, BOOL alt_down) {
    UINT scan = MapVirtualKeyA(vk, MAPVK_VK_TO_VSC);
    LPARAM l = 1 | ((LPARAM)scan << 16);

    if (alt_down) {
        l |= ((LPARAM)1 << 29);
    }
    if (key_up) {
        l |= ((LPARAM)1 << 30) | ((LPARAM)1 << 31);
    }
    return l;
}

static void send_modifier_key(UINT vk, BOOL key_down, BOOL alt_down) {
    LPARAM l = key_lparam_from_vk(vk, key_down ? FALSE : TRUE, alt_down);
    SendMessageA(ime_target_hwnd(), key_down ? WM_KEYDOWN : WM_KEYUP, (WPARAM)vk, l);
}

static int handle_key_mod_internal(
    unsigned vk,
    BOOL ctrl,
    BOOL shift,
    BOOL alt,
    char *out,
    size_t out_size,
    BOOL activate_first) {
    BYTE key_state[256];
    TRANSMSGLIST_COMPAT tml;
    HIMC ctx;
    HIMC use_himc;
    UINT scan;
    LPARAM key_data;
    LPARAM key_up_data;
    UINT i;
    HKL hkl;
    DWORD thread_id;
    DWORD proc_id;
    UINT ime_vk;

    ZeroMemory(&tml, sizeof(tml));

    if (!GetKeyboardState(key_state)) {
        ZeroMemory(key_state, sizeof(key_state));
    }
    if (ctrl) {
        key_state[VK_CONTROL] |= 0x80;
    }
    if (shift) {
        key_state[VK_SHIFT] |= 0x80;
    }
    if (alt) {
        key_state[VK_MENU] |= 0x80;
    }
    if (vk < 256) {
        key_state[vk] |= 0x80;
    }

    if (activate_first) {
        activate_ime_path();
    }
    ctx = ImmGetContext(ime_target_hwnd());
    g_host.last_ctx_now = ctx;
    use_himc = ctx ? ctx : g_host.himc;

    scan = MapVirtualKeyA(vk, MAPVK_VK_TO_VSC);
    key_data = key_lparam_from_vk(vk, FALSE, alt);
    key_up_data = key_lparam_from_vk(vk, TRUE, alt);

    if (ctrl) {
        send_modifier_key(VK_CONTROL, TRUE, alt);
    }
    if (shift) {
        send_modifier_key(VK_SHIFT, TRUE, alt);
    }
    if (alt) {
        send_modifier_key(VK_MENU, TRUE, TRUE);
    }

    SendMessageA(ime_target_hwnd(), WM_KEYDOWN, (WPARAM)vk, key_data);

    proc_id = 0;
    thread_id = GetWindowThreadProcessId(ime_target_hwnd(), &proc_id);
    hkl = GetKeyboardLayout(thread_id);
    g_host.last_imm_flags = ImmProcessKey(ime_target_hwnd(), hkl, vk, key_data, IME_INVALID_HOTKEY);
    if (g_host.last_imm_flags & IPHK_PROCESSBYIME) {
        ImmTranslateMessage(ime_target_hwnd(), WM_KEYDOWN, (WPARAM)vk, key_data);
    }

    /* Let IMM/IME posted handlers settle state before direct export probing. */
    pump_messages_once();

    ime_vk = make_ime_vkey(vk, scan, key_state, hkl);
    g_host.last_process = g_host.ime_process_key(use_himc, vk, key_data, key_state);
    g_host.last_ascii = g_host.ime_to_ascii_ex(ime_vk, scan, key_state, &tml, 0, use_himc);
    g_host.last_msgs = tml.uMsgCount;

    for (i = 0; i < tml.uMsgCount && i < 16; ++i) {
        SendMessageA(ime_target_hwnd(), tml.TransMsg[i].message, tml.TransMsg[i].wParam, tml.TransMsg[i].lParam);
    }

    SendMessageA(ime_target_hwnd(), WM_KEYUP, (WPARAM)vk, key_up_data);

    if (alt) {
        send_modifier_key(VK_MENU, FALSE, TRUE);
    }
    if (shift) {
        send_modifier_key(VK_SHIFT, FALSE, alt);
    }
    if (ctrl) {
        send_modifier_key(VK_CONTROL, FALSE, alt);
    }

    pump_messages_once();

    g_host.last_notify = 0;

    if (ctx) {
        ImmReleaseContext(ime_target_hwnd(), ctx);
    }

    {
        BOOL borrowed = FALSE;
        HIMC cand_ctx = get_active_himc(&borrowed);
        if (cand_ctx) {
            CANDIDATELIST *cand = NULL;
            DWORD bytes = ImmGetCandidateListW(cand_ctx, 0, NULL, 0);
            g_host.last_comp_bytes = ImmGetCompositionStringW(cand_ctx, GCS_COMPSTR, NULL, 0);
            if (bytes) {
                cand = (CANDIDATELIST *)malloc(bytes);
                if (cand && ImmGetCandidateListW(cand_ctx, 0, cand, bytes)) {
                    g_host.last_cand_count = cand->dwCount;
                    g_host.last_cand_sel = cand->dwSelection;
                } else {
                    g_host.last_cand_count = 0;
                    g_host.last_cand_sel = 0;
                }
                if (cand) {
                    free(cand);
                }
            } else {
                g_host.last_cand_count = 0;
                g_host.last_cand_sel = 0;
            }
            release_active_himc(cand_ctx, borrowed);
        }
    }

    if (tml.uMsgCount > 0) {
        snprintf(
            out,
            out_size,
            "KEY_RET process=%d immFlags=0x%08lx ascii=%u msgs=%u scan=0x%02x imeVk=0x%08x firstMsg=0x%04x firstW=0x%08lx firstL=0x%08lx\n",
            (int)g_host.last_process,
            (unsigned long)g_host.last_imm_flags,
            (unsigned)g_host.last_ascii,
            (unsigned)g_host.last_msgs,
            (unsigned)scan,
            (unsigned)ime_vk,
            (unsigned)tml.TransMsg[0].message,
            (unsigned long)tml.TransMsg[0].wParam,
            (unsigned long)tml.TransMsg[0].lParam);
    } else {
        snprintf(
            out,
            out_size,
            "KEY_RET process=%d immFlags=0x%08lx ascii=%u msgs=%u scan=0x%02x imeVk=0x%08x\n",
            (int)g_host.last_process,
            (unsigned long)g_host.last_imm_flags,
            (unsigned)g_host.last_ascii,
            (unsigned)g_host.last_msgs,
            (unsigned)scan,
            (unsigned)ime_vk);
    }

    return 0;
}

static int handle_key_internal(unsigned vk, char *out, size_t out_size, BOOL activate_first) {
    return handle_key_mod_internal(vk, FALSE, FALSE, FALSE, out, out_size, activate_first);
}

static int handle_key(unsigned vk, char *out, size_t out_size) {
    return handle_key_internal(vk, out, out_size, TRUE);
}

static int run_key_and_reply_cand(unsigned vk, char *out, size_t out_size) {
    handle_key_internal(vk, out, out_size, FALSE);
    return reply_with_candidate_snapshot(out, out_size);
}

static void append_trace_step(const char *key_label, char *out, size_t out_size, size_t *off) {
    BOOL borrowed = FALSE;
    HIMC himc = get_active_himc(&borrowed);
    LONG comp_bytes = 0;
    LONG read_bytes = 0;
    WCHAR comp_w[96];
    WCHAR read_w[96];
    char comp_utf8[128];
    char read_utf8[128];
    DWORD list_count = 0;
    DWORD total_size = 0;
    DWORD c0 = 0;
    DWORD sel0 = 0;
    DWORD page0 = 0;

    if (himc) {
        DWORD bytes = 0;
        int is_ansi = 0;
        CANDIDATELIST *cand = NULL;
        DWORD got = 0;

        comp_bytes = read_comp_wide(himc, GCS_COMPSTR, comp_w, sizeof(comp_w) / sizeof(comp_w[0]));
        read_bytes = read_comp_wide(himc, GCS_COMPREADSTR, read_w, sizeof(read_w) / sizeof(read_w[0]));
        decode_ime_wide_text(comp_w, comp_utf8, sizeof(comp_utf8));
        decode_ime_wide_text(read_w, read_utf8, sizeof(read_utf8));

        total_size = ImmGetCandidateListCountW(himc, &list_count);
        if (!list_count) {
            total_size = ImmGetCandidateListCountA(himc, &list_count);
        }

        bytes = ImmGetCandidateListW(himc, 0, NULL, 0);
        if (bytes) {
            is_ansi = 0;
        } else {
            bytes = ImmGetCandidateListA(himc, 0, NULL, 0);
            is_ansi = 1;
        }

        if (bytes) {
            cand = (CANDIDATELIST *)malloc(bytes);
            if (cand) {
                got = is_ansi ? ImmGetCandidateListA(himc, 0, cand, bytes)
                              : ImmGetCandidateListW(himc, 0, cand, bytes);
                if (got && got <= bytes) {
                    c0 = cand->dwCount;
                    sel0 = cand->dwSelection;
                    page0 = cand->dwPageSize;
                }
                free(cand);
            }
        }
    } else {
        comp_utf8[0] = 0;
        read_utf8[0] = 0;
    }

    appendf(
        out,
        out_size,
        off,
        "%s{proc=%d imm=0x%08lx ascii=%u msgs=%u compBytes=%ld readBytes=%ld comp=[%s] read=[%s] lists=%lu total=%lu c0=%lu sel=%lu page=%lu}",
        key_label,
        (int)g_host.last_process,
        (unsigned long)g_host.last_imm_flags,
        (unsigned)g_host.last_ascii,
        (unsigned)g_host.last_msgs,
        comp_bytes > 0 ? comp_bytes : 0,
        read_bytes > 0 ? read_bytes : 0,
        comp_utf8,
        read_utf8,
        (unsigned long)list_count,
        (unsigned long)total_size,
        (unsigned long)c0,
        (unsigned long)sel0,
        (unsigned long)page0);

    release_active_himc(himc, borrowed);
}

static int run_trace_ascii(const char *text, BOOL add_space_trigger, char *out, size_t out_size) {
    const char *p = text;
    HKL hkl;
    size_t off = 0;
    unsigned steps = 0;
    int wrote = 0;

    while (*p == ' ' || *p == '\t') {
        ++p;
    }

    if (!*p) {
        snprintf(out, out_size, "TRACE_RET err=empty\n");
        return 0;
    }

    activate_ime_path();
    hkl = get_target_hkl();
    appendf(out, out_size, &off, "TRACE_RET cp=%u data=", (unsigned)g_host.cand_codepage);

    while (*p && steps < 24) {
        KeyChord chord;
        if (key_chord_from_ascii_char(*p, hkl, &chord)) {
            char key_label[16];
            char key_reply[256];
            if (steps > 0) {
                appendf(out, out_size, &off, ";");
            }

            if (*p == ' ') {
                lstrcpynA(key_label, "SP", (int)sizeof(key_label));
            } else if (isprint((unsigned char)*p) && *p != ';' && *p != '[' && *p != ']') {
                snprintf(key_label, sizeof(key_label), "%c", *p);
            } else {
                snprintf(key_label, sizeof(key_label), "0x%02X", (unsigned char)*p);
            }

            handle_key_mod_internal(
                chord.vk,
                chord.ctrl,
                chord.shift,
                chord.alt,
                key_reply,
                sizeof(key_reply),
                FALSE);
            if ((unsigned char)*p >= 0x20 && (unsigned char)*p < 0x7f) {
                SendMessageA(ime_target_hwnd(), WM_CHAR, (WPARAM)(unsigned char)*p, 1);
                pump_messages_once();
            }
            append_trace_step(key_label, out, out_size, &off);
            wrote = 1;
            ++steps;
        }
        ++p;
    }

    if (add_space_trigger && steps < 24) {
        char key_reply[256];
        if (steps > 0) {
            appendf(out, out_size, &off, ";");
        }
        handle_key_internal(VK_SPACE, key_reply, sizeof(key_reply), FALSE);
        append_trace_step("SPC*", out, out_size, &off);
        wrote = 1;
        ++steps;
    }

    if (!wrote) {
        snprintf(out, out_size, "TRACE_RET err=no_mapped_keys\n");
        return 0;
    }

    appendf(out, out_size, &off, "\n");
    return 0;
}

static int run_trace_wide(const WCHAR *text, BOOL add_space_trigger, char *out, size_t out_size) {
    const WCHAR *p = text;
    BOOL old_pref = g_host.prefer_unicode_vkey;
    HKL hkl;
    size_t off = 0;
    unsigned steps = 0;
    int wrote = 0;

    g_host.prefer_unicode_vkey = TRUE;

    while (*p == L' ' || *p == L'\t') {
        ++p;
    }

    if (!*p) {
        g_host.prefer_unicode_vkey = old_pref;
        snprintf(out, out_size, "TRACE_RET err=empty\n");
        return 0;
    }

    activate_ime_path();
    hkl = get_target_hkl();
    appendf(out, out_size, &off, "TRACE_RET cp=%u data=", (unsigned)g_host.cand_codepage);

    while (*p && steps < 24) {
        KeyChord chord;
        int mapped = key_chord_from_wide_char(*p, hkl, &chord);
        int printable = (*p >= 0x20 && *p != 0x7f);

        if (mapped || printable) {
            char key_label[32];
            char key_reply[256];

            if (steps > 0) {
                appendf(out, out_size, &off, ";");
            }

            key_label_from_wide_char(*p, key_label, sizeof(key_label));

            if (mapped) {
                handle_key_mod_internal(
                    chord.vk,
                    chord.ctrl,
                    chord.shift,
                    chord.alt,
                    key_reply,
                    sizeof(key_reply),
                    FALSE);
            } else {
                g_host.last_process = 0;
                g_host.last_imm_flags = 0;
                g_host.last_ascii = 0;
                g_host.last_msgs = 0;
            }

            if (printable) {
                SendMessageW(ime_target_hwnd(), WM_CHAR, (WPARAM)(unsigned)*p, 1);
                pump_messages_once();
            }

            append_trace_step(key_label, out, out_size, &off);
            wrote = 1;
            ++steps;
        }

        ++p;
    }

    if (add_space_trigger && steps < 24) {
        char key_reply[256];
        if (steps > 0) {
            appendf(out, out_size, &off, ";");
        }
        handle_key_internal(VK_SPACE, key_reply, sizeof(key_reply), FALSE);
        append_trace_step("SPC*", out, out_size, &off);
        wrote = 1;
        ++steps;
    }

    if (!wrote) {
        g_host.prefer_unicode_vkey = old_pref;
        snprintf(out, out_size, "TRACE_RET err=no_mapped_keys\n");
        return 0;
    }

    appendf(out, out_size, &off, "\n");
    g_host.prefer_unicode_vkey = old_pref;
    return 0;
}

static int run_trace_utf8(const char *text, BOOL add_space_trigger, char *out, size_t out_size) {
    WCHAR wide[256];
    int n = utf8_to_wide_text(text, wide, sizeof(wide) / sizeof(wide[0]));

    if (n < 0) {
        snprintf(out, out_size, "TRACE_RET err=bad_utf8\n");
        return 0;
    }

    return run_trace_wide(wide, add_space_trigger, out, out_size);
}

static int run_text_wide(const WCHAR *text, BOOL add_space_trigger, BOOL pump_after_char, char *out, size_t out_size) {
    const WCHAR *p = text;
    BOOL old_pref = g_host.prefer_unicode_vkey;
    HKL hkl;
    unsigned sent = 0;

    g_host.prefer_unicode_vkey = TRUE;

    activate_ime_path();
    hkl = get_target_hkl();
    while (*p && sent < 64) {
        KeyChord chord;
        int mapped = key_chord_from_wide_char(*p, hkl, &chord);
        int printable = (*p >= 0x20 && *p != 0x7f);

        if (mapped) {
            handle_key_mod_internal(
                chord.vk,
                chord.ctrl,
                chord.shift,
                chord.alt,
                out,
                out_size,
                FALSE);
        }

        if (printable) {
            SendMessageW(ime_target_hwnd(), WM_CHAR, (WPARAM)(unsigned)*p, 1);
            if (pump_after_char) {
                pump_messages_once();
            }
        }

        if (mapped || printable) {
            ++sent;
        }

        ++p;
    }

    if (add_space_trigger) {
        /* Space is the common convert/confirm trigger in many IMEs. */
        handle_key_internal(VK_SPACE, out, out_size, FALSE);
    }

    reply_with_candidate_snapshot(out, out_size);
    g_host.prefer_unicode_vkey = old_pref;
    return 0;
}

static int run_text_utf8(const char *text, BOOL add_space_trigger, BOOL pump_after_char, char *out, size_t out_size) {
    WCHAR wide[512];
    int n = utf8_to_wide_text(text, wide, sizeof(wide) / sizeof(wide[0]));

    if (n < 0) {
        snprintf(out, out_size, "CAND_RET err=bad_utf8\n");
        return 0;
    }

    return run_text_wide(wide, add_space_trigger, pump_after_char, out, out_size);
}

static int run_keytext_ascii(const char *text, BOOL add_space_trigger, char *out, size_t out_size) {
    const char *p = text;
    HKL hkl;
    unsigned sent = 0;

    activate_ime_path();
    hkl = get_target_hkl();

    while (*p && sent < 64) {
        KeyChord chord;
        if (key_chord_from_ascii_char(*p, hkl, &chord)) {
            handle_key_mod_internal(
                chord.vk,
                chord.ctrl,
                chord.shift,
                chord.alt,
                out,
                out_size,
                FALSE);
            ++sent;
        }
        ++p;
    }

    if (add_space_trigger) {
        handle_key_internal(VK_SPACE, out, out_size, FALSE);
    }

    reply_with_candidate_snapshot(out, out_size);
    return 0;
}

static int run_keytext_wide(const WCHAR *text, BOOL add_space_trigger, char *out, size_t out_size) {
    const WCHAR *p = text;
    HKL hkl;
    unsigned sent = 0;

    activate_ime_path();
    hkl = get_target_hkl();

    while (*p && sent < 64) {
        KeyChord chord;
        if (key_chord_from_wide_char(*p, hkl, &chord)) {
            handle_key_mod_internal(
                chord.vk,
                chord.ctrl,
                chord.shift,
                chord.alt,
                out,
                out_size,
                FALSE);
            ++sent;
        }
        ++p;
    }

    if (add_space_trigger) {
        handle_key_internal(VK_SPACE, out, out_size, FALSE);
    }

    reply_with_candidate_snapshot(out, out_size);
    return 0;
}

static int run_keytext_utf8(const char *text, BOOL add_space_trigger, char *out, size_t out_size) {
    WCHAR wide[512];
    int n = utf8_to_wide_text(text, wide, sizeof(wide) / sizeof(wide[0]));

    if (n < 0) {
        snprintf(out, out_size, "CAND_RET err=bad_utf8\n");
        return 0;
    }

    return run_keytext_wide(wide, add_space_trigger, out, out_size);
}

static int process_command(const char *line) {
    char reply[2048];

    if (strncmp(line, "PING", 4) == 0) {
        return send_text(g_host.client_sock, "PONG\n");
    }

    if (strncmp(line, "STATUS", 6) == 0) {
        snprintf(
            reply,
            sizeof(reply),
            "STATUS visible=%d target=%p select=%d activate=%d open=%d convSet=%d conv=0x%08lx sent=0x%08lx hkl=%p hklSwitch=%d ctxMatch=%d assocPrev=%p ctxNow=%p process=%d immFlags=0x%08lx ascii=%u msgs=%u cand=%lu candSel=%lu compBytes=%ld candCP=%u notify=%d prop=0x%08lx uiClass=%ls err=%s\n",
            (int)g_host.show_window,
            ime_target_hwnd(),
            (int)g_host.last_select,
            (int)g_host.last_activate,
            (int)g_host.last_open,
            (int)g_host.last_conv_set,
            (unsigned long)g_host.last_conv_mode,
            (unsigned long)g_host.last_sentence_mode,
            g_host.last_hkl,
            (int)g_host.last_hkl_switch,
            (int)g_host.last_ctx_match,
            g_host.last_assoc_prev,
            g_host.last_ctx_now,
            (int)g_host.last_process,
            (unsigned long)g_host.last_imm_flags,
            (unsigned)g_host.last_ascii,
            (unsigned)g_host.last_msgs,
            (unsigned long)g_host.last_cand_count,
            (unsigned long)g_host.last_cand_sel,
            (long)g_host.last_comp_bytes,
            (unsigned)g_host.cand_codepage,
            (int)g_host.last_notify,
            (unsigned long)g_host.info.fdwProperty,
            g_host.ui_class,
            g_host.last_error[0] ? g_host.last_error : "none");
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "CP", 2) == 0 && (line[2] == 0 || line[2] == ' ' || line[2] == '\t')) {
        const char *p = line + 2;
        unsigned cp = 0;

        while (*p == ' ' || *p == '\t') {
            ++p;
        }

        if (!*p) {
            snprintf(reply, sizeof(reply), "OK CP %u\n", (unsigned)g_host.cand_codepage);
            return send_text(g_host.client_sock, reply);
        }

        if (sscanf(p, "%u", &cp) != 1 || cp == 0) {
            return send_text(g_host.client_sock, "ERR bad codepage\n");
        }

        g_host.cand_codepage = cp;
        snprintf(reply, sizeof(reply), "OK CP %u\n", cp);
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "CAND", 4) == 0) {
        reply_with_candidate_snapshot(reply, sizeof(reply));
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "PREEDIT", 7) == 0) {
        BOOL borrowed = FALSE;
        HIMC himc = get_active_himc(&borrowed);
        build_preedit_reply(himc, reply, sizeof(reply));
        release_active_himc(himc, borrowed);
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "TRACEPIPE ", 10) == 0) {
        run_trace_ascii(line + 10, TRUE, reply, sizeof(reply));
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "TRACEPIPEU ", 11) == 0) {
        run_trace_utf8(line + 11, TRUE, reply, sizeof(reply));
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "TRACE ", 6) == 0) {
        run_trace_ascii(line + 6, FALSE, reply, sizeof(reply));
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "TRACEU ", 7) == 0) {
        run_trace_utf8(line + 7, FALSE, reply, sizeof(reply));
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "PAGEDOWN", 8) == 0) {
        run_key_and_reply_cand(VK_NEXT, reply, sizeof(reply));
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "PAGEUP", 6) == 0) {
        run_key_and_reply_cand(VK_PRIOR, reply, sizeof(reply));
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "PICK ", 5) == 0) {
        unsigned pick = 0;
        unsigned vk = 0;

        if (sscanf(line + 5, "%u", &pick) != 1 || pick > 9) {
            return send_text(g_host.client_sock, "ERR bad pick\n");
        }

        vk = (pick == 0) ? '0' : ('0' + pick);
        run_key_and_reply_cand(vk, reply, sizeof(reply));
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "COMMIT", 6) == 0) {
        BOOL borrowed = FALSE;
        HIMC himc;

        himc = get_active_himc(&borrowed);
        if (g_host.notify_ime && himc) {
            g_host.last_notify = g_host.notify_ime(himc, NI_COMPOSITIONSTR, CPS_COMPLETE, 0);
            snprintf(reply, sizeof(reply), "OK COMMIT notify=%d\n", (int)g_host.last_notify);
        } else {
            g_host.last_notify = 0;
            snprintf(reply, sizeof(reply), "OK COMMIT notify=0\n");
        }
        release_active_himc(himc, borrowed);
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "KEYTEXT ", 8) == 0) {
        run_keytext_ascii(line + 8, FALSE, reply, sizeof(reply));
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "KEYPIPE ", 8) == 0) {
        run_keytext_ascii(line + 8, TRUE, reply, sizeof(reply));
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "KEYTEXTU ", 9) == 0) {
        run_keytext_utf8(line + 9, FALSE, reply, sizeof(reply));
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "KEYPIPEU ", 9) == 0) {
        run_keytext_utf8(line + 9, TRUE, reply, sizeof(reply));
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "TEXT ", 5) == 0) {
        const char *p = line + 5;
        HKL hkl;
        unsigned sent = 0;

        activate_ime_path();
        hkl = get_target_hkl();
        while (*p && sent < 64) {
            KeyChord chord;
            if (key_chord_from_ascii_char(*p, hkl, &chord)) {
                handle_key_mod_internal(
                    chord.vk,
                    chord.ctrl,
                    chord.shift,
                    chord.alt,
                    reply,
                    sizeof(reply),
                    FALSE);
                if ((unsigned char)*p >= 0x20 && (unsigned char)*p < 0x7f) {
                    SendMessageA(ime_target_hwnd(), WM_CHAR, (WPARAM)(unsigned char)*p, 1);
                    pump_messages_once();
                }
                ++sent;
            }
            ++p;
        }

        reply_with_candidate_snapshot(reply, sizeof(reply));
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "TEXTU ", 6) == 0) {
        run_text_utf8(line + 6, FALSE, TRUE, reply, sizeof(reply));
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "PIPE ", 5) == 0) {
        const char *p = line + 5;
        HKL hkl;
        unsigned sent = 0;

        activate_ime_path();
        hkl = get_target_hkl();
        while (*p && sent < 64) {
            KeyChord chord;
            if (key_chord_from_ascii_char(*p, hkl, &chord)) {
                handle_key_mod_internal(
                    chord.vk,
                    chord.ctrl,
                    chord.shift,
                    chord.alt,
                    reply,
                    sizeof(reply),
                    FALSE);
                if ((unsigned char)*p >= 0x20 && (unsigned char)*p < 0x7f) {
                    SendMessageA(ime_target_hwnd(), WM_CHAR, (WPARAM)(unsigned char)*p, 1);
                }
                ++sent;
            }
            ++p;
        }

        /* Space is the common convert/confirm trigger in many IMEs. */
        handle_key_internal(VK_SPACE, reply, sizeof(reply), FALSE);

        reply_with_candidate_snapshot(reply, sizeof(reply));
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "PIPEU ", 6) == 0) {
        run_text_utf8(line + 6, TRUE, FALSE, reply, sizeof(reply));
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "CONV", 4) == 0) {
        BOOL borrowed = FALSE;
        HIMC himc;
        WCHAR source[128];
        DWORD source_bytes = 0;
        const char *arg = line + 4;

        while (*arg == ' ' || *arg == '\t') {
            ++arg;
        }

        himc = get_active_himc(&borrowed);
        if (!g_host.ime_conversion_list || !himc) {
            release_active_himc(himc, borrowed);
            return send_text(g_host.client_sock, "CONV_RET err=unsupported\n");
        }

        ZeroMemory(source, sizeof(source));
        if (*arg) {
            int n = MultiByteToWideChar(CP_UTF8, 0, arg, -1, source, (int)(sizeof(source) / sizeof(source[0])));
            if (n <= 0) {
                release_active_himc(himc, borrowed);
                return send_text(g_host.client_sock, "CONV_RET err=bad_text\n");
            }
            source_bytes = (DWORD)((n - 1) * sizeof(WCHAR));
        } else {
            LONG comp_bytes = ImmGetCompositionStringW(himc, GCS_COMPSTR, source, sizeof(source) - sizeof(WCHAR));
            if (comp_bytes > 0) {
                source_bytes = (DWORD)comp_bytes;
                source[comp_bytes / (LONG)sizeof(WCHAR)] = 0;
            }
        }

        if (!source_bytes) {
            release_active_himc(himc, borrowed);
            return send_text(g_host.client_sock, "CONV_RET err=no_source\n");
        }

        {
            UINT bytes = g_host.ime_conversion_list(himc, source, NULL, 0, 0);
            if (!bytes) {
                release_active_himc(himc, borrowed);
                return send_text(g_host.client_sock, "CONV_RET count=0 bytes=0\n");
            }

            {
                CANDIDATELIST *cand = (CANDIDATELIST *)malloc(bytes);
                if (!cand) {
                    release_active_himc(himc, borrowed);
                    return send_text(g_host.client_sock, "CONV_RET err=oom\n");
                }

                if (!g_host.ime_conversion_list(himc, source, cand, bytes, 0)) {
                    free(cand);
                    release_active_himc(himc, borrowed);
                    return send_text(g_host.client_sock, "CONV_RET err=read_fail\n");
                }

                {
                    size_t off = 0;
                    DWORD i;
                    appendf(reply, sizeof(reply), &off, "CONV_RET count=%lu sel=%lu items=[",
                            (unsigned long)cand->dwCount,
                            (unsigned long)cand->dwSelection);
                    for (i = 0; i < cand->dwCount && i < 10; ++i) {
                        const WCHAR *w;
                        char utf8[192];
                        int j;

                        if (cand->dwOffset[i] >= bytes) {
                            continue;
                        }
                        w = (const WCHAR *)((const BYTE *)cand + cand->dwOffset[i]);
                        if (!WideCharToMultiByte(CP_UTF8, 0, w, -1, utf8, (int)sizeof(utf8), NULL, NULL)) {
                            lstrcpynA(utf8, "<conv_err>", (int)sizeof(utf8));
                        }
                        for (j = 0; utf8[j]; ++j) {
                            if (utf8[j] == '|' || utf8[j] == '\r' || utf8[j] == '\n') {
                                utf8[j] = ' ';
                            }
                        }
                        if (i > 0) {
                            appendf(reply, sizeof(reply), &off, "|");
                        }
                        appendf(reply, sizeof(reply), &off, "%s", utf8);
                    }
                    if (cand->dwCount > 10) {
                        appendf(reply, sizeof(reply), &off, "|...");
                    }
                    appendf(reply, sizeof(reply), &off, "]\n");
                }

                free(cand);
            }
        }

        release_active_himc(himc, borrowed);
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "KEY ", 4) == 0) {
        unsigned vk = 0;
        if (sscanf(line + 4, "%x", &vk) != 1) {
            return send_text(g_host.client_sock, "ERR bad key\n");
        }
        handle_key(vk, reply, sizeof(reply));
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "KEYCHORD ", 9) == 0) {
        unsigned vk = 0;
        unsigned ctrl = 0;
        unsigned shift = 0;
        unsigned alt = 0;
        int n = sscanf(line + 9, "%x %u %u %u", &vk, &ctrl, &shift, &alt);
        if (n < 1) {
            return send_text(g_host.client_sock, "ERR bad keychord\n");
        }
        handle_key_mod_internal(vk, ctrl ? TRUE : FALSE, shift ? TRUE : FALSE, alt ? TRUE : FALSE, reply, sizeof(reply), TRUE);
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "RESET", 5) == 0) {
        if (reset_context() == 0) {
            return send_text(g_host.client_sock, "OK RESET\n");
        }
        snprintf(reply, sizeof(reply), "ERR RESET %s\n", g_host.last_error);
        return send_text(g_host.client_sock, reply);
    }

    if (strncmp(line, "ACTIVATE", 8) == 0) {
        activate_ime_path();
        return send_text(g_host.client_sock, "OK ACTIVATE\n");
    }

    if (strncmp(line, "QUIT", 4) == 0) {
        send_text(g_host.client_sock, "BYE\n");
        g_host.running = FALSE;
        return 0;
    }

    return send_text(g_host.client_sock, "ERR unknown command\n");
}

static void process_rx_buffer(void) {
    int start = 0;
    int i;

    for (i = 0; i < g_host.rx_len; ++i) {
        if (g_host.rx_buf[i] == '\n') {
            char line[512];
            int len = i - start;
            int j;

            if (len >= (int)sizeof(line)) {
                len = (int)sizeof(line) - 1;
            }
            for (j = 0; j < len; ++j) {
                line[j] = g_host.rx_buf[start + j];
            }
            line[len] = 0;

            while (len > 0 && (line[len - 1] == '\r' || line[len - 1] == ' ' || line[len - 1] == '\t')) {
                line[--len] = 0;
            }

            if (len > 0) {
                process_command(line);
            }
            start = i + 1;
        }
    }

    if (start > 0) {
        int remain = g_host.rx_len - start;
        if (remain > 0) {
            memmove(g_host.rx_buf, g_host.rx_buf + start, (size_t)remain);
        }
        g_host.rx_len = remain;
    }
}

static void service_client_io(void) {
    char tmp[1024];
    int n;

    if (g_host.client_sock == INVALID_SOCKET) {
        return;
    }

    n = recv(g_host.client_sock, tmp, sizeof(tmp), 0);
    if (n == 0) {
        close_client();
        return;
    }

    if (n < 0) {
        int err = WSAGetLastError();
        if (err == WSAEWOULDBLOCK) {
            return;
        }
        close_client();
        return;
    }

    if (g_host.rx_len + n > RX_BUF_SIZE) {
        close_client();
        return;
    }

    memcpy(g_host.rx_buf + g_host.rx_len, tmp, (size_t)n);
    g_host.rx_len += n;
    process_rx_buffer();
}

static int init_socket_server(unsigned short port) {
    WSADATA wsa;
    struct sockaddr_in addr;
    u_long nonblock = 1;

    if (WSAStartup(MAKEWORD(2, 2), &wsa) != 0) {
        set_last_errorf("WSAStartup failed");
        return -1;
    }

    g_host.listen_sock = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (g_host.listen_sock == INVALID_SOCKET) {
        set_last_errorf("socket failed: %d", WSAGetLastError());
        return -1;
    }

    ioctlsocket(g_host.listen_sock, FIONBIO, &nonblock);

    ZeroMemory(&addr, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    addr.sin_port = htons(port);

    if (bind(g_host.listen_sock, (struct sockaddr *)&addr, sizeof(addr)) != 0) {
        set_last_errorf("bind failed: %d", WSAGetLastError());
        return -1;
    }

    if (listen(g_host.listen_sock, 1) != 0) {
        set_last_errorf("listen failed: %d", WSAGetLastError());
        return -1;
    }

    return 0;
}

static void accept_client_if_needed(void) {
    struct sockaddr_in addr;
    int len = sizeof(addr);
    u_long nonblock = 1;
    SOCKET s;

    if (g_host.client_sock != INVALID_SOCKET) {
        return;
    }

    s = accept(g_host.listen_sock, (struct sockaddr *)&addr, &len);
    if (s == INVALID_SOCKET) {
        return;
    }

    ioctlsocket(s, FIONBIO, &nonblock);
    g_host.client_sock = s;
    g_host.rx_len = 0;
    send_text(g_host.client_sock, "HELLO ime_host_skeleton\n");
}

static void cleanup(void) {
    close_client();

    if (g_host.listen_sock != INVALID_SOCKET) {
        closesocket(g_host.listen_sock);
        g_host.listen_sock = INVALID_SOCKET;
    }

    if (g_host.himc) {
        if (g_host.ime_select) {
            g_host.ime_select(g_host.himc, FALSE);
        }
        ImmAssociateContextEx(ime_target_hwnd(), NULL, 0);
        ImmDestroyContext(g_host.himc);
        g_host.himc = NULL;
    }

    if (g_host.hwnd) {
        DestroyWindow(g_host.hwnd);
        g_host.hwnd = NULL;
    }

    if (g_host.ime) {
        FreeLibrary(g_host.ime);
        g_host.ime = NULL;
    }

    WSACleanup();
}

int main(int argc, char **argv) {
    const char *dll_path = DEFAULT_DLL_PATH;
    unsigned short port = DEFAULT_PORT;
    int i;

    ZeroMemory(&g_host, sizeof(g_host));
    g_host.listen_sock = INVALID_SOCKET;
    g_host.client_sock = INVALID_SOCKET;
    g_host.cand_codepage = 936;

    for (i = 1; i < argc; ++i) {
        if (strcmp(argv[i], "--dll") == 0 && i + 1 < argc) {
            dll_path = argv[++i];
        } else if (strcmp(argv[i], "--port") == 0 && i + 1 < argc) {
            port = (unsigned short)atoi(argv[++i]);
        } else if (strcmp(argv[i], "--show-window") == 0) {
            g_host.show_window = TRUE;
        }
    }

    if (init_socket_server(port) != 0) {
        printf("[host] socket init failed: %s\n", g_host.last_error);
        cleanup();
        return 1;
    }

    if (init_ime(dll_path) != 0) {
        printf("[host] ime init failed: %s\n", g_host.last_error);
        cleanup();
        return 2;
    }

    printf("[host] started\n");
    printf("[host] dll=%s\n", dll_path);
    printf("[host] port=%u\n", (unsigned)port);
    printf("[host] uiClass=%ls\n", g_host.ui_class);
    printf("[host] visible=%d\n", (int)g_host.show_window);
    printf("[host] status select=%d\n", (int)g_host.last_select);
    printf("[host] commands: PING | STATUS | CP [codepage] | ACTIVATE | KEY <hex_vk> | KEYCHORD <hex_vk> [ctrl] [shift] [alt] | KEYTEXT <ascii> | KEYTEXTU <utf8> | KEYPIPE <ascii> | KEYPIPEU <utf8> | CAND | PREEDIT | TRACE <ascii> | TRACEPIPE <ascii> | TRACEU <utf8> | TRACEPIPEU <utf8> | PAGEDOWN | PAGEUP | PICK <0..9> | TEXT <ascii> | TEXTU <utf8> | PIPE <ascii> | PIPEU <utf8> | CONV [utf8] | COMMIT | RESET | QUIT\n");

    g_host.running = TRUE;
    while (g_host.running) {
        pump_messages_once();
        accept_client_if_needed();
        service_client_io();
        Sleep(10);
    }

    cleanup();
    return 0;
}
