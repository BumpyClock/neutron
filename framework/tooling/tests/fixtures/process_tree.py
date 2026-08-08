#!/usr/bin/env python3
"""Process-tree fixture for Stage 1 supervisor integration tests."""

from __future__ import annotations

import argparse
import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import subprocess
import sys
import time


def write_pid(path: Path | None) -> None:
    if path is not None:
        path.write_text(f"{os.getpid()}\n", encoding="utf-8")


def windows_membership() -> dict[str, bool]:
    if os.name != "nt":
        return {"job_member": False, "unrelated_handle_inherited": False}
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.GetCurrentProcess.restype = wintypes.HANDLE
    kernel32.IsProcessInJob.argtypes = [
        wintypes.HANDLE,
        wintypes.HANDLE,
        ctypes.POINTER(wintypes.BOOL),
    ]
    kernel32.IsProcessInJob.restype = wintypes.BOOL
    kernel32.GetHandleInformation.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
    kernel32.GetHandleInformation.restype = wintypes.BOOL

    in_job = wintypes.BOOL()
    if not kernel32.IsProcessInJob(kernel32.GetCurrentProcess(), None, ctypes.byref(in_job)):
        raise ctypes.WinError(ctypes.get_last_error())

    inherited = False
    unrelated = os.environ.get("STAGE1_UNRELATED_HANDLE")
    if unrelated is not None:
        flags = wintypes.DWORD()
        inherited = bool(
            kernel32.GetHandleInformation(wintypes.HANDLE(int(unrelated)), ctypes.byref(flags))
        )
    return {
        "job_member": bool(in_job.value),
        "unrelated_handle_inherited": inherited,
    }


def spawn_grandchild(pid_file: Path | None) -> subprocess.Popen[bytes]:
    command = [sys.executable, __file__, "grandchild"]
    if pid_file is not None:
        command.extend(["--pid-file", str(pid_file)])
    return subprocess.Popen(command)


def main() -> int:
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument(
        "mode",
        choices=("success", "exit", "membership", "grandchild", "spawn-wait", "spawn-exit"),
    )
    parser.add_argument("--pid-file", type=Path)
    parser.add_argument("--release-file", type=Path)
    parser.add_argument("--exit-code", type=int, default=0)
    args = parser.parse_args()

    if args.mode == "success":
        sys.stdout.write("stage1 stdout complete\n")
        sys.stdout.flush()
        sys.stderr.write("stage1 stderr complete\n")
        sys.stderr.flush()
        return 0
    if args.mode == "exit":
        sys.stdout.write(f"declared exit {args.exit_code}\n")
        sys.stdout.flush()
        return args.exit_code
    if args.mode == "membership":
        sys.stdout.write(json.dumps(windows_membership(), sort_keys=True) + "\n")
        sys.stdout.flush()
        return 0
    if args.mode == "grandchild":
        write_pid(args.pid_file)
        time.sleep(300)
        return 0

    grandchild = spawn_grandchild(args.pid_file)
    sys.stdout.write(f"grandchild_pid={grandchild.pid}\n")
    sys.stdout.flush()
    if args.mode == "spawn-exit":
        if args.pid_file is not None:
            deadline = time.monotonic() + 5
            while not args.pid_file.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            if not args.pid_file.exists():
                sys.stderr.write("grandchild did not publish its PID before root exit\n")
                return 2
        if args.release_file is not None:
            deadline = time.monotonic() + 5
            while not args.release_file.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            if not args.release_file.exists():
                sys.stderr.write("root exit was not released\n")
                return 3
        return 0
    grandchild.wait()
    return grandchild.returncode


if __name__ == "__main__":
    sys.exit(main())
