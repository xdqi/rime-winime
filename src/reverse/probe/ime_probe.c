#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <stdio.h>
#include <wchar.h>

typedef struct _IMEINFO_COMPAT {
    DWORD dwPrivateDataSize;
    DWORD fdwProperty;
    DWORD fdwConversionCaps;
    DWORD fdwSentenceCaps;
    DWORD fdwUICaps;
    DWORD fdwSCSCaps;
    DWORD fdwSelectCaps;
} IMEINFO_COMPAT;

typedef BOOL (WINAPI *PFN_ImeInquire)(IMEINFO_COMPAT *lpIMEInfo, LPWSTR lpszUIClass, DWORD dwSystemInfoFlags);
typedef LRESULT (WINAPI *PFN_ImeEscape)(HIMC hIMC, UINT uSubFunc, LPVOID lpData);
typedef BOOL (WINAPI *PFN_ImeSelect)(HIMC hIMC, BOOL fSelect);
typedef BOOL (WINAPI *PFN_ImeProcessKey)(HIMC hIMC, UINT vKey, LPARAM lKeyData, const BYTE *keyState);
typedef UINT (WINAPI *PFN_ImeToAsciiEx)(UINT vKey, UINT scanCode, const BYTE *keyState, void *transBuf, UINT state, HIMC hIMC);
typedef BOOL (WINAPI *PFN_NotifyIME)(HIMC hIMC, DWORD action, DWORD index, DWORD value);

static void print_export(HMODULE mod, const char *name) {
    FARPROC p = GetProcAddress(mod, name);
    printf("%-22s : %p\n", name, (void *)p);
}

int main(int argc, char **argv) {
    const char *dll_path = "C:\\windows\\system32\\SogouPY.ime";
    if (argc > 1) {
        dll_path = argv[1];
    }

    printf("[probe] loading: %s\n", dll_path);
    HMODULE mod = LoadLibraryA(dll_path);
    if (!mod) {
        DWORD err = GetLastError();
        printf("[probe] LoadLibraryA failed, err=%lu\n", (unsigned long)err);
        return 1;
    }

    puts("[probe] exported addresses:");
    print_export(mod, "ImeInquire");
    print_export(mod, "ImeEscape");
    print_export(mod, "ImeSelect");
    print_export(mod, "ImeProcessKey");
    print_export(mod, "ImeToAsciiEx");
    print_export(mod, "NotifyIME");

    PFN_ImeInquire ime_inquire = (PFN_ImeInquire)GetProcAddress(mod, "ImeInquire");
    PFN_ImeEscape ime_escape = (PFN_ImeEscape)GetProcAddress(mod, "ImeEscape");

    if (ime_inquire) {
        IMEINFO_COMPAT info;
        WCHAR ui_class[64];
        ZeroMemory(&info, sizeof(info));
        ZeroMemory(ui_class, sizeof(ui_class));

        BOOL ok = ime_inquire(&info, ui_class, 0);
        printf("[probe] ImeInquire ret=%d\n", (int)ok);
        wprintf(L"[probe] UI class: %ls\n", ui_class);
        printf("[probe] IMEINFO fdwProperty=0x%08lx fdwConversionCaps=0x%08lx fdwSentenceCaps=0x%08lx\n",
               (unsigned long)info.fdwProperty,
               (unsigned long)info.fdwConversionCaps,
               (unsigned long)info.fdwSentenceCaps);
    } else {
        puts("[probe] ImeInquire not found");
    }

    if (ime_escape) {
        WCHAR out_text[64];
        ZeroMemory(out_text, sizeof(out_text));
        LRESULT r = ime_escape(NULL, 4102, out_text);
        printf("[probe] ImeEscape(4102) ret=%ld\n", (long)r);
        wprintf(L"[probe] ImeEscape text: %ls\n", out_text);
    } else {
        puts("[probe] ImeEscape not found");
    }

    FreeLibrary(mod);
    puts("[probe] done");
    return 0;
}
