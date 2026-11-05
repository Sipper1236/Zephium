"""The startup probe must reject a window which immediately disconnects."""

import contextlib
import io
import itertools
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

import probe_linux_startup as probe


class StartupProbeTests(unittest.TestCase):
    def run_probe(self, returncode, output, backend="x11", current_session=False):
        process = Mock(pid=12345, returncode=returncode)
        process.poll.return_value = returncode
        clock = itertools.count(step=0.5)
        with tempfile.TemporaryDirectory() as directory:
            with (
                patch.object(probe.subprocess, "Popen", return_value=process) as launch,
                patch.object(probe.subprocess, "run") as native_calls,
                patch.object(probe.shutil, "which", return_value="/usr/bin/fusermount3"),
                patch.object(probe, "contents", return_value=output),
                patch.object(probe.time, "monotonic", side_effect=lambda: next(clock)),
                patch.object(probe.time, "sleep"),
                patch.object(probe.os, "killpg") as kill,
                contextlib.redirect_stdout(io.StringIO()),
            ):
                try:
                    probe.probe(Path("/checkout/zephium-desktop"), backend, Path(directory), 10, 2,
                                current_session=current_session)
                finally:
                    kill.assert_called_once()
                    process.wait.assert_called_once()
                    self.assertEqual(
                        [call.args[0] for call in native_calls.call_args_list],
                        [["/usr/bin/fusermount3", "-u", str(Path(directory) / "runtime" / name)]
                         for name in ("doc", "gvfs")],
                    )
        return launch.call_args

    def test_ready_marker_before_clean_exit_is_not_success(self):
        with self.assertRaisesRegex(RuntimeError, "exited early"):
            self.run_probe(0, probe.READY)

    def test_ready_marker_before_crash_is_not_success(self):
        with self.assertRaisesRegex(RuntimeError, "exited early"):
            self.run_probe(1, probe.READY)

    def test_process_without_initialized_window_times_out(self):
        with self.assertRaisesRegex(RuntimeError, "deadline"):
            self.run_probe(None, "")

    def test_live_initialized_window_is_isolated_from_the_user_session(self):
        call = self.run_probe(None, probe.READY)
        self.assertEqual(call.args[0][-1], "/checkout/zephium-desktop")
        self.assertEqual(call.args[0][:2], ["dbus-run-session", "--"])
        self.assertTrue(call.kwargs["start_new_session"])
        env = call.kwargs["env"]
        self.assertEqual(env["GDK_BACKEND"], "x11")
        self.assertNotIn("DBUS_SESSION_BUS_ADDRESS", env)
        self.assertNotIn("DISPLAY", env)
        self.assertNotIn("WAYLAND_DISPLAY", env)
        self.assertTrue(env["XDG_DATA_HOME"].endswith("/data"))
        self.assertTrue(env["XDG_STATE_HOME"].endswith("/state"))
        self.assertTrue(env["HOME"].endswith("/home"))

    def test_current_wayland_session_uses_display_socket_with_disposable_data(self):
        with patch.dict(probe.os.environ, {
            "WAYLAND_DISPLAY": "wayland-1", "XDG_RUNTIME_DIR": "/run/user/1000",
            "DBUS_SESSION_BUS_ADDRESS": "unix:path=/run/user/1000/bus",
        }):
            call = self.run_probe(None, probe.READY, "wayland", True)
        self.assertEqual(call.args[0], ["dbus-run-session", "--", "/checkout/zephium-desktop"])
        env = call.kwargs["env"]
        self.assertEqual(env["WAYLAND_DISPLAY"], "/run/user/1000/wayland-1")
        self.assertNotEqual(env["XDG_RUNTIME_DIR"], "/run/user/1000")
        self.assertNotIn("DBUS_SESSION_BUS_ADDRESS", env)
        self.assertTrue(env["XDG_DATA_HOME"].endswith("/data"))

    def test_current_x11_session_uses_actual_display(self):
        with patch.dict(probe.os.environ, {"DISPLAY": ":42", "HOME": "/home/person"}, clear=True):
            call = self.run_probe(None, probe.READY, "x11", True)
        self.assertEqual(call.kwargs["env"]["DISPLAY"], ":42")
        self.assertEqual(call.kwargs["env"]["XAUTHORITY"], "/home/person/.Xauthority")
        self.assertNotEqual(call.kwargs["env"]["HOME"], "/home/person")
        self.assertNotIn("xvfb-run", call.args[0])

    def test_current_x11_session_preserves_explicit_authority(self):
        with patch.dict(probe.os.environ, {"DISPLAY": ":42", "XAUTHORITY": "/run/user/1000/xauth"}):
            call = self.run_probe(None, probe.READY, "x11", True)
        self.assertEqual(call.kwargs["env"]["XAUTHORITY"], "/run/user/1000/xauth")


if __name__ == "__main__":
    unittest.main()
