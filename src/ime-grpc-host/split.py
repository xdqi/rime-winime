import os

filepath = "/opt/sogou/src/ime-grpc-host/src/backend/win_imm.rs"
with open(filepath, 'r') as f:
    lines = f.readlines()

windows_lines = []
not_windows_lines = []

in_win = False
in_not_win = False
skip_next = False

common_lines = []

# We actually need to parse this properly. 
# Better: just grep the methods and manually write a script that separates them. 
