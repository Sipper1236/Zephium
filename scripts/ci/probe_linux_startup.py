#!/usr/bin/env python3
"""Launch this checkout with disposable data on a test or current desktop."""

import argparse
import os
from pathlib import Path
import signal
import shutil
import subprocess
import tempfile
import time


READY = "main document initialized and window shown"


def stop(process):
    """Reap only the process group created by this probe."""
    if process is None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=5)


def contents(path):
    if not path.exists():
        return ""
    with path.open("rb") as stream:
        stream.seek(max(0, path.stat().st_size - 65536))
        return stream.read(65536).decode("utf-8", errors="replace")


def probe(executable, backend, root, timeout, settle, current_session=False):
    runtime = root / "runtime"
    runtime.mkdir(mode=0o700)
    env = dict(os.environ)
    for key in ("DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "APPIMAGE", "APPDIR"):
        env.pop(key, None)
    env.update(
        GDK_BACKEND=backend,
        XDG_RUNTIME_DIR=str(runtime),
        XDG_CONFIG_HOME=str(root / "config"),
        XDG_DATA_HOME=str(root / "data"),
        XDG_CACHE_HOME=str(root / "cache"),
        XDG_STATE_HOME=str(root / "state"),
        HOME=str(root / "home"),
        XDG_CURRENT_DESKTOP="WESTON" if backend == "wayland" else "X-Generic",
        XDG_SESSION_TYPE=backend,
        # Avoid a session-activated gvfs FUSE mount inside the disposable
        # runtime directory: it can outlive the private bus during cleanup.
        GIO_USE_VFS="local",
    )
    if current_session:
        # Keep profile data disposable, but connect to the actual display.
        # A private bus still prevents single-instance forwarding to a running
        # user instance and avoids prompting the user's portal during a probe.
        if backend == "wayland":
            display = os.environ.get("WAYLAND_DISPLAY")
            session_runtime = os.environ.get("XDG_RUNTIME_DIR")
            if not display or not session_runtime:
                raise RuntimeError("current Wayland session has no display/runtime directory")
            env["WAYLAND_DISPLAY"] = str(Path(session_runtime) / display)
        else:
            display = os.environ.get("DISPLAY")
            if not display:
                raise RuntimeError("current X11 session has no DISPLAY")
            env["DISPLAY"] = display
            env["XAUTHORITY"] = os.environ.get("XAUTHORITY") or str(Path.home() / ".Xauthority")
        env["XDG_CURRENT_DESKTOP"] = os.environ.get("XDG_CURRENT_DESKTOP", "")
    (root / "home").mkdir()
    # Portal host registration requires the canonical desktop entry, even for
    # a source checkout. Install it only inside the disposable data directory.
    applications = root / "data" / "applications"
    applications.mkdir(parents=True)
    quoted = str(executable).replace("\\", "\\\\\\\\")
    for character in ('"', '`', '$'):
        quoted = quoted.replace(character, "\\\\" + character)
    quoted = quoted.replace("%", "%%")
    (applications / "app.zephium.desktop").write_text(
        "[Desktop Entry]\nType=Application\nName=Zephium\n"
        f'Exec="{quoted}" %U\nIcon=app.zephium\n',
        encoding="utf-8",
    )
    compositor = None
    application = None
    compositor_log = root / "display.log"
    application_log = root / "application.log"
    try:
        if current_session:
            command = [str(executable)]
        elif backend == "wayland":
            env["WAYLAND_DISPLAY"] = "zephium-probe"
            command = [
                "weston", "--backend=headless-backend.so", "--use-pixman",
                "--shell=kiosk-shell.so", "--no-config", "--idle-time=0",
                "--socket=zephium-probe", "--width=1024", "--height=640",
            ]
            # Recent Weston can provide a keyboard even without physical input.
            help_text = subprocess.run(
                ["weston", "--help"], capture_output=True, text=True, check=True,
            ).stdout
            if "--fake-seat" in help_text:
                command.append("--fake-seat")
            with compositor_log.open("wb") as log:
                compositor = subprocess.Popen(
                    command, env=env, stdout=log, stderr=log, start_new_session=True,
                )
            deadline = time.monotonic() + 10
            while not (runtime / "zephium-probe").is_socket():
                if compositor.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("Wayland compositor did not become ready")
                time.sleep(0.05)
            command = [str(executable)]
        else:
            command = [
                "xvfb-run", "-a", "-s", "-screen 0 1280x800x24",
                str(executable),
            ]
        with application_log.open("wb") as log:
            application = subprocess.Popen(
                ["dbus-run-session", "--", *command], env=env,
                stdout=log, stderr=log, start_new_session=True,
            )
        deadline = time.monotonic() + timeout
        ready_at = None
        while time.monotonic() < deadline:
            if application.poll() is not None:
                raise RuntimeError(f"application exited early ({application.returncode})")
            if compositor is not None and compositor.poll() is not None:
                raise RuntimeError("Wayland compositor exited early")
            if ready_at is None and READY in contents(application_log):
                ready_at = time.monotonic()
            if ready_at is not None and time.monotonic() - ready_at >= settle:
                print(f"{backend}: initialized window remained alive for {settle:g}s")
                return
            time.sleep(0.1)
        raise RuntimeError("application did not initialize before the deadline")
    except (OSError, RuntimeError, subprocess.SubprocessError):
        print(contents(application_log))
        print(contents(compositor_log))
        raise
    finally:
        try:
            stop(application)
        finally:
            stop(compositor)
            # Portal/GVfs services activated on the private bus may leave
            # disconnected FUSE mounts after the session ends. Unmount only
            # these two known mountpoints inside this probe's private root.
            unmount = shutil.which("fusermount3") or shutil.which("fusermount")
            if unmount:
                for name in ("doc", "gvfs"):
                    subprocess.run(
                        [unmount, "-u", str(runtime / name)],
                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                        timeout=5, check=False,
                    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("executable", type=Path)
    parser.add_argument("--backend", choices=("x11", "wayland"), required=True)
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--settle", type=float, default=5)
    parser.add_argument(
        "--current-session", action="store_true",
        help="show a disposable-profile window on the current physical desktop",
    )
    args = parser.parse_args()
    if args.settle <= 0 or args.timeout <= args.settle:
        parser.error("require timeout > settle > 0")
    executable = args.executable.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="zephium-linux-probe-") as directory:
        probe(executable, args.backend, Path(directory), args.timeout, args.settle,
              current_session=args.current_session)


if __name__ == "__main__":
    main()
