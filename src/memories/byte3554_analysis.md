# byte_3554 (offset 0xDE2) Comprehensive Analysis - SogouPY 64-bit

## Version Detection Chain
- sub_1801B6D70 → sub_1801BECD0 (kernel32.dll file version + RtlGetNtVersionNumbers)
- byte_3554 = 1 when: major > 6 OR (major == 6 AND minor >= 2) → Win8+ (6.2+)

## All ImeToAsciiEx Path References

### 1. sub_180154360 (main ImeToAsciiEx) - TWO references
- **Ref 1 - SetDataToIMC guard**: When byte_3554==0 (pre-Win8), writes to COMPOSITIONSTRING before processing via virtual call SetDataToContext. When byte_3554==1 (Win8+), this is SKIPPED.
- **Ref 2 - PostMessage fallback**: When byte_3554==0, checks byte_3640 + sub_1801BE510(), and if conditions met, PostMessages each TRANSMSG directly. Skipped when byte_3554==1.

### 2. sub_180153D30 (ImeProcessKey) - ONE reference
- **SetDataToIMC guard**: Same pattern as sub_180154360 Ref 1, but with additional guard byte_3582==1.

### 3. sub_180154DF0 (NotifyIME handler, contains 0x155173) - ONE reference
- **Composition state reset**: When byte_3554!=0 OR byte_3581==1, skips composition state toggle logic in IMN_COMPOSITIONSTRING.

### 4. sub_1802F8A60 (TRANSMSG dispatch) - TWO references (two paths)
- **Core dispatch switch**: byte_3554==1 → SendMessageW(hWnd, 0x8BB8, count, data). byte_3554==0 → write to hMsgBuf + ImmGenerateMessage(hIMC).

## CRITICAL FINDING: byte_3554 controls COMPOSITIONSTRING writing
YES - not just SendMessage dispatch. When byte_3554==0, the code performs "CL_SetDataToIMCC" / "CL_SetDataToContext" which writes data to INPUTCONTEXT before key processing. When byte_3554==1, this pre-synchronization is skipped.

## COMPOSITIONSTRING Writing: sub_18014CB70
This is THE function that writes engine data → INPUTCONTEXT after key processing.
Called at 0x1801547BF in ImeToAsciiEx, BEFORE sub_18014D1A0 (TRANSMSG dispatch).
Called regardless of byte_3554.

Flow inside sub_18014CB70:
1. ImmLockIMC
2. Scan command list for type-0 commands with case 3/6/8
3. IF found AND hPrivate.vtable[7](count) AND hPrivate.vtable[5]():
   → Call hCompStr.vtable[48] (offset 0x180) = populate result string
4. UNCONDITIONALLY: SetDataToIMCC for hCompStr, hCandInfo, hPrivate
5. SetDataToContext for INPUTCONTEXT context fields
6. ImmUnlockIMC

### Why case 3 works, case 8 doesn't:
- Case 3 (nihao+space): active composition → hPrivate conditions pass → vtable[48] populates result
- Case 8 (standalone punct): no prior composition → hPrivate conditions fail → vtable[48] skipped

## Offset Map
- 8: Legacy pre-Vista flag (XP/2K)
- 9: Vista+ flag (major >= 6)
- 10: Running in iexplore.exe (Vista+)
- 11: Win10+ flag
- 3554 (0xDE2): Win8+ flag - controls IMC sync + message dispatch
- 3574 (0xDF6): Controls TRANSMSG template size for cases 6/10 in dispatch
- 3579 (0xDFB): In TRANSMSG processing, controls context flag
- 3581 (0xDFD): Additional guard in NotifyIME composition handling
- 3582 (0xDFE): Additional guard in ImeProcessKey SetDataToIMC
- 3624 (0xE28): Post-command PostMessage(0x83FA) trigger
- 3633 (0xE31): IME_SETOPEN SendMessage trigger
- 3640 (0xE38): PostMessage fallback condition (used with byte_3554==0)
