#!/usr/bin/env python3
"""Serialize cargo builds so Foundry FFI end-to-end calls do not race."""
import fcntl
import os
import subprocess
import sys


def main() -> int:
    if len(sys.argv) < 2:
        print("missing script", file=sys.stderr)
        return 1
    here = os.path.abspath(os.path.dirname(__file__))
    repo = os.path.abspath(os.path.join(here, "..", "..", "..", ".."))
    workspace = os.path.join(repo, "programs", "solevm-remote-leg")
    binary = os.path.join(workspace, "target", "debug", "solevm-e2e")
    lock_path = os.path.join(workspace, "target", "solevm-e2e.lock")
    os.makedirs(os.path.dirname(lock_path), exist_ok=True)
    with open(lock_path, "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        subprocess.check_call(
            [
                "cargo",
                "build",
                "--quiet",
                "--manifest-path",
                os.path.join(workspace, "e2e", "Cargo.toml"),
                "--bin",
                "solevm-e2e",
            ]
        )
    os.execv(binary, [binary, sys.argv[1]])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
