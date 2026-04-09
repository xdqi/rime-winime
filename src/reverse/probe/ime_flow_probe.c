#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <imm.h>
#include <stdio.h>

typedef struct _TRANSMSG_COMPAT {
    UINT message;
    WPARAM wParam;
    LPARAM lParam;
} TRANSMSG_COMPAT;

typedef struct _TRANSMSGLIST_COMPAT {
    UINT uMsgCount;
    TRANSMSG_COMPAT TransMsg[16];
} TRANSMSGLIST_COMPAT;

typedef BOOL (WINAPI *PFN_ImeSelect)(HIMC hIMC, BOOL fSelect);
typedef BOOL (WINAPI *PFN_ImeProcessKey)(HIMC hIMC, UINT vKey, LPARAM lKeyData, const BYTE *keyState);
typedef UINT (WINAPI *PFN_ImeToAsciiEx)(UINT vKey, UINT scanCode, const BYTE *keyState, void *transBuf, UINT state, HIMC hIMC);
typedef BOOL (WINAPI *PFN_NotifyIME)(HIMC hIMC, DWORD action, DWORD index, DWORD value);

#define VK_A_KEY 0x41

static void print_msg_list(const TRANSMSGLIST_COMPAT *list) {
    UINT i;
    UINT n = list->uMsgCount;
    if (n > 4) {
        n = 4;
    }
    for (i = 0; i < n; ++i) {
        printf("[flow] msg[%u] message=0x%04x wParam=0x%08lx lParam=0x%08lx\n",
               i,
               (unsigned)list->TransMsg[i].message,
               (unsigned long)list->TransMsg[i].wParam,
               (unsigned long)list->TransMsg[i].lParam);
    }
}

int main(int argc, char **argv) {
    const char *dll_path = "C:\\windows\\system32\\SogouPY.ime";
    BYTE key_state[256];
    HMODULE mod;
    HIMC himc;
    PFN_ImeSelect ime_select;
    PFN_ImeProcessKey ime_process_key;
    PFN_ImeToAsciiEx ime_to_ascii_ex;
    PFN_NotifyIME notify_ime;
    BOOL r_select;
    BOOL r_process;
    UINT r_ascii;
    TRANSMSGLIST_COMPAT tml;

    if (argc > 1) {
        dll_path = argv[1];
    }

    printf("[flow] loading: %s\n", dll_path);
    mod = LoadLibraryA(dll_path);
    if (!mod) {
        printf("[flow] LoadLibraryA failed, err=%lu\n", (unsigned long)GetLastError());
        return 1;
    }

    ime_select = (PFN_ImeSelect)GetProcAddress(mod, "ImeSelect");
    ime_process_key = (PFN_ImeProcessKey)GetProcAddress(mod, "ImeProcessKey");
    ime_to_ascii_ex = (PFN_ImeToAsciiEx)GetProcAddress(mod, "ImeToAsciiEx");
    notify_ime = (PFN_NotifyIME)GetProcAddress(mod, "NotifyIME");

    if (!ime_select || !ime_process_key || !ime_to_ascii_ex) {
        puts("[flow] required exports missing");
        FreeLibrary(mod);
        return 2;
    }

    himc = ImmCreateContext();
    printf("[flow] ImmCreateContext -> %p\n", himc);
    if (!himc) {
        FreeLibrary(mod);
        return 3;
    }

    r_select = ime_select(himc, TRUE);
    printf("[flow] ImeSelect(TRUE) -> %d\n", (int)r_select);

    ZeroMemory(key_state, sizeof(key_state));
    key_state[VK_A_KEY] = 0x80;

    r_process = ime_process_key(himc, VK_A_KEY, 0, key_state);
    printf("[flow] ImeProcessKey(VK_A) -> %d\n", (int)r_process);

    ZeroMemory(&tml, sizeof(tml));
    r_ascii = ime_to_ascii_ex(VK_A_KEY, 0, key_state, &tml, 0, himc);
    printf("[flow] ImeToAsciiEx(VK_A) -> %u, uMsgCount=%u\n", r_ascii, tml.uMsgCount);
    print_msg_list(&tml);

    if (notify_ime) {
        BOOL r_notify = notify_ime(himc, NI_COMPOSITIONSTR, CPS_COMPLETE, 0);
        printf("[flow] NotifyIME(NI_COMPOSITIONSTR,CPS_COMPLETE) -> %d\n", (int)r_notify);
    }

    ime_select(himc, FALSE);
    ImmDestroyContext(himc);
    FreeLibrary(mod);
    puts("[flow] done");
    return 0;
}
