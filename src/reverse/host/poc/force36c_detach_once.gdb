set pagination off
set confirm off
set breakpoint pending on

set $mod_base = 0
set $added = 0
set $ime_mod = 0
set $writes = 0

break init_context_and_select
commands
silent
if $added == 0
set $ime_mod = *(unsigned int*)__HOST_IME_PTR_ADDR__
if $ime_mod == 0
continue
end
set $mod_base = $ime_mod
set $added = 1
printf "MOD_BASE=0x%x\n", $mod_base

break *($mod_base + 0xf60b5)
commands 2
silent
set $obj = *(unsigned int*)($esi + 0xc)
set *(unsigned char*)($obj + 0x36c) = 1
set $writes = $writes + 1
printf "FORCE_36C_ONCE[%d] obj=0x%x b36c=%u\n", $writes, $obj, *(unsigned char*)($obj + 0x36c)
detach
quit
end

end
continue
end

target remote localhost:__GDB_PORT__
continue