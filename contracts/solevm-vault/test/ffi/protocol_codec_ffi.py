#!/usr/bin/env python3
"""Serialize cargo builds so Foundry FFI codec checks do not race."""
import fcntl
import os
import subprocess
import sys

def main() -> int:
    if len(sys.argv) < 2:
        print("missing command", file=sys.stderr)
        return 1
    here = os.path.abspath(os.path.dirname(__file__))
    repo = os.path.abspath(os.path.join(here, "..", "..", "..", ".."))
    target = os.path.join(repo, "target")
    os.makedirs(target, exist_ok=True)
    binary = os.path.join(target, "debug", "protocol-codec-ffi")
    lock_path = os.path.join(target, "protocol-codec-ffi.lock")
    with open(lock_path, "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if not os.path.isfile(binary):
            subprocess.check_call(
                [
                    "cargo",
                    "build",
                    "--quiet",
                    "--manifest-path",
                    os.path.join(repo, "crates", "protocol-types", "Cargo.toml"),
                    "--bin",
                    "protocol-codec-ffi",
                    "--features",
                    "std",
                ]
            )
    os.execv(binary, [binary, *sys.argv[1:]])
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
