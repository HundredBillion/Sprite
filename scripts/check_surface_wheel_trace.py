#!/usr/bin/env python3
"""Check the isolated five-event wheel gesture in a Linux strace log."""

import re
import sys
from pathlib import Path


def check(trace):
    lines = trace.splitlines()
    starts = [i for i, line in enumerate(lines) if "SURFACE_GESTURE_BEGIN" in line]
    ends = [i for i, line in enumerate(lines) if "SURFACE_GESTURE_END" in line]
    if len(starts) != 1 or len(ends) != 1 or starts[0] >= ends[0]:
        raise ValueError("expected exactly one complete gesture")
    calls = [line for line in lines[starts[0] + 1 : ends[0]]
             if re.search(r"\b(sendto|sendmsg|write|writev)\(", line)]
    if len(calls) != 1:
        raise ValueError(f"wheel syscall budget exceeded: {len(calls)} != 1")
    if not re.search(r"= 399$", calls[0]):
        raise ValueError("the normal wheel write must deliver all 399 bytes")
    if calls[0].count(r'\"type\":\"mouse\"') != 5 or calls[0].count(r"\n") != 5:
        raise ValueError("trace must contain five complete JSON mouse lines; use strace -s 2048")
    print("wheel gesture: 1 syscall, 399 bytes, 5 complete JSON lines")


if __name__ == "__main__":
    try:
        if len(sys.argv) != 2:
            raise ValueError("usage: check_surface_wheel_trace.py TRACE")
        check(Path(sys.argv[1]).read_text())
    except (ValueError, OSError) as error:
        sys.exit(str(error))
