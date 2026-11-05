#!/usr/bin/env python3
"""Exercise native vault fixtures on a private bus and disposable keyring."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time


def fixture(binary, name, env):
    available = subprocess.run([str(binary), "--list"], env=env,
                               check=True, capture_output=True, text=True, timeout=10)
    if f"{name}: test" not in available.stdout.splitlines():
        raise RuntimeError(f"credential test binary is missing fixture {name}")
    subprocess.run([str(binary), name, "--exact", "--ignored", "--test-threads=1"],
                   env=env, check=True, timeout=60)


def session(binary, root):
    env = os.environ.copy()
    env.update(HOME=str(root), XDG_DATA_HOME=str(root / "data"),
               XDG_CONFIG_HOME=str(root / "config"), XDG_CACHE_HOME=str(root / "cache"),
               XDG_RUNTIME_DIR=str(root / "runtime"))
    Path(env["XDG_RUNTIME_DIR"]).mkdir(mode=0o700)
    control = root / "control"
    control.mkdir(mode=0o700)
    with (root / "keyring.log").open("w+") as log:
        daemon = subprocess.Popen(
            ["gnome-keyring-daemon", "--foreground", "--unlock", "--components=secrets",
             f"--control-directory={control}"], env=env, stdin=subprocess.PIPE,
            stdout=log, stderr=subprocess.STDOUT,
        )
        try:
            # A nonsecret password protects only disposable, nonsecret test fixtures.
            daemon.stdin.write(b"zephium-native-test-fixtures\n")
            daemon.stdin.close()
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                if daemon.poll() is not None:
                    raise RuntimeError("isolated keyring exited before registration")
                owner = subprocess.run(
                    ["dbus-send", "--session", "--dest=org.freedesktop.DBus", "--print-reply",
                     "/", "org.freedesktop.DBus.NameHasOwner", "string:org.freedesktop.secrets"],
                    env=env, capture_output=True, text=True, timeout=5,
                )
                if owner.returncode == 0 and "boolean true" in owner.stdout:
                    break
                time.sleep(0.1)
            else:
                raise RuntimeError("isolated keyring did not register")
            owner = subprocess.run(
                ["dbus-send", "--session", "--dest=org.freedesktop.DBus", "--print-reply",
                 "/", "org.freedesktop.DBus.GetConnectionUnixProcessID", "string:org.freedesktop.secrets"],
                env=env, capture_output=True, text=True, check=True, timeout=5,
            )
            owner_pid = re.search(r"\buint32\s+(\d+)", owner.stdout)
            if not owner_pid or int(owner_pid.group(1)) != daemon.pid:
                raise RuntimeError("Secret Service is not the disposable keyring started by this probe")
            fixture(binary, "linux::tests::native_roundtrip_replacement_and_exact_deletion", env)
            default = subprocess.run(
                ["dbus-send", "--session", "--dest=org.freedesktop.secrets", "--print-reply",
                 "/org/freedesktop/secrets", "org.freedesktop.Secret.Service.ReadAlias", "string:default"],
                env=env, check=True, capture_output=True, text=True, timeout=5,
            )
            collection = re.search(r'object path "([^"]+)"', default.stdout)
            if not collection or collection.group(1) == "/":
                raise RuntimeError("disposable keyring has no default collection to retire")
            # GNOME Keyring may resolve an unset default alias back to login;
            # retire the actual disposable collection to test its absence.
            subprocess.run(
                ["dbus-send", "--session", "--dest=org.freedesktop.secrets", "--print-reply",
                 collection.group(1), "org.freedesktop.Secret.Collection.Delete"],
                env=env, check=True, capture_output=True, text=True, timeout=5,
            )
            fixture(binary, "linux::tests::missing_default_collection_fails_closed", env)
        finally:
            daemon.terminate()
            try:
                daemon.wait(timeout=5)
            except subprocess.TimeoutExpired:
                daemon.kill()
                daemon.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test-binary", type=Path)
    parser.add_argument("--session-root", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.session_root:
        session(args.test_binary.resolve(), args.session_root)
        return
    binary = args.test_binary
    if binary is None:
        build = subprocess.run(["cargo", "test", "-p", "zephium-credentials", "--no-run",
                                "--locked", "--message-format=json"],
                               check=True, capture_output=True, text=True)
        artifacts = [json.loads(line) for line in build.stdout.splitlines()]
        binary = next(Path(item["executable"]) for item in artifacts
                      if item.get("reason") == "compiler-artifact"
                      and item.get("executable") and item.get("profile", {}).get("test"))
    with tempfile.TemporaryDirectory(prefix="zephium-vault-") as directory:
        unavailable_env = os.environ.copy()
        unavailable_env["DBUS_SESSION_BUS_ADDRESS"] = f"unix:path={directory}/absent-bus"
        fixture(binary.resolve(), "linux::tests::unavailable_service_fails_closed", unavailable_env)
        subprocess.run(["dbus-run-session", "--", sys.executable, str(Path(__file__).resolve()),
                        "--test-binary", str(binary.resolve()), "--session-root", directory],
                       check=True, timeout=100)


if __name__ == "__main__":
    main()
