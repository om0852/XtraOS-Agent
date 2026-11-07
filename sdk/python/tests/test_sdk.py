"""Python SDK against a private XtraOS core (no windows touched, no input)."""
import os
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from xtraos import XtraOS, XtraOSError  # noqa: E402

ROOT = Path(__file__).resolve().parents[3]
EXE = Path(os.environ.get("XTRAOS_CORE_EXE", ROOT / "core" / "target" / "release" / "xtraos-core.exe"))


@unittest.skipUnless(EXE.exists(), f"build the core first: {EXE}")
class SdkTest(unittest.TestCase):
    def setUp(self) -> None:
        self.home = tempfile.mkdtemp(prefix="xtraos-sdkpy-")
        self.pipe = "\\\\.\\pipe\\xtraos-sdkpy-" + secrets.token_hex(4)
        self.token = secrets.token_hex(32)
        env = dict(os.environ, XTRAOS_HOME=self.home, XTRAOS_PIPE=self.pipe, XTRAOS_TOKEN=self.token)
        self.core = subprocess.Popen([str(EXE)], env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        out = b""
        deadline = time.monotonic() + 15
        while b"READY" not in out and time.monotonic() < deadline:
            out += self.core.stdout.read1(256)

    def tearDown(self) -> None:
        self.core.kill()
        self.core.wait(5)
        self.core.stdout.close()
        shutil.rmtree(self.home, ignore_errors=True)

    def test_sdk_p1_calls_and_errors(self) -> None:
        with XtraOS(pipe=self.pipe, token=self.token, client="sdk-py-test") as x:
            st = x.call("status")
            self.assertEqual(st["state"], "running")
            self.assertEqual(st["client"], "sdk-py-test")
            self.assertIsInstance(x.call("list_windows")["windows"], list)
            with self.assertRaises(XtraOSError) as e:
                x.call("volume", op="louder")
            self.assertEqual(e.exception.code, "INVALID_ARGS")
        with self.assertRaises(XtraOSError):
            XtraOS(pipe=self.pipe, token="f" * 64, autostart=False).call("status")


if __name__ == "__main__":
    unittest.main()
