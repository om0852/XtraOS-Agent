"""xtraos: Python client for the XtraOS Agent core (ADR 0042). Standard library only, Windows.

It pairs with the user's XtraOS core exactly like the MCP server and the CLI, so the same
policy, approval dialogs, kill switch and action log apply.

    from xtraos import XtraOS
    with XtraOS() as x:
        print(x.call("volume", op="get")["text"])
"""
from __future__ import annotations

import json
import os
import subprocess
import time
from pathlib import Path
from typing import Any, Optional

__all__ = ["XtraOS", "XtraOSError"]
__version__ = "0.1.0"


class XtraOSError(Exception):
    """A contract error from the core: INVALID_ARGS, BLOCKED_BY_POLICY, NEEDS_HUMAN, …"""

    def __init__(self, code: str, message: str, hint: str = "") -> None:
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message
        self.hint = hint


def _home() -> Path:
    h = os.environ.get("XTRAOS_HOME")
    return Path(h) if h else Path(os.environ.get("LOCALAPPDATA", ".")) / "XtraOS"


def _core_exe() -> Optional[Path]:
    p = os.environ.get("XTRAOS_CORE_EXE")
    exe = Path(p) if p else Path(os.environ.get("LOCALAPPDATA", ".")) / "Programs" / "XtraOS" / "bin" / "xtraos-core.exe"
    return exe if exe.exists() else None


class XtraOS:
    """Client for one XtraOS core. `call(tool, **params)` returns the tool's result dict."""

    def __init__(
        self,
        pipe: Optional[str] = None,
        token: Optional[str] = None,
        client: str = "xtraos-sdk-python",
        autostart: bool = True,
    ) -> None:
        self._pipe_opt, self._token_opt = pipe, token
        self._client = client
        self._autostart = autostart and not pipe and not os.environ.get("XTRAOS_PIPE")
        self._f = None
        self._buf = b""
        self._id = 0

    # -- connection ---------------------------------------------------------------------
    def _resolve(self) -> tuple[Optional[str], Optional[str]]:
        pipe = self._pipe_opt or os.environ.get("XTRAOS_PIPE")
        if not pipe:
            try:
                pipe = json.loads((_home() / "connection.json").read_text("utf-8"))["pipe"]
            except (OSError, ValueError, KeyError):
                pipe = None
        token = self._token_opt or os.environ.get("XTRAOS_TOKEN")
        if not token:
            try:
                token = (_home() / "pairing.token").read_text("utf-8").strip()
            except OSError:
                token = None
        return pipe, token

    def _connect_once(self) -> None:
        pipe, token = self._resolve()
        if not pipe or not token:
            raise XtraOSError("NOT_PAIRED", "XtraOS core connection info or token not found")
        try:
            self._f = open(pipe, "r+b", buffering=0)
        except OSError as e:
            raise XtraOSError("NOT_PAIRED", f"XtraOS core not reachable ({e})") from None
        self._buf = b""
        try:
            self._rpc("pair", {"token": token, "client": self._client})
        except XtraOSError:
            self.close()
            raise

    def _connect(self) -> None:
        if self._f is not None:
            return
        try:
            self._connect_once()
            return
        except XtraOSError:
            exe = _core_exe() if self._autostart else None
            if exe is None:
                raise
        # Detached, hidden, no inherited handles: the core lives in the user's session.
        flags = 0x00000008 | 0x08000000  # DETACHED_PROCESS | CREATE_NO_WINDOW
        subprocess.Popen(
            [str(exe)],
            creationflags=flags,
            close_fds=True,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        deadline = time.monotonic() + 10
        while True:
            time.sleep(0.2)
            try:
                self._connect_once()
                return
            except XtraOSError:
                if time.monotonic() > deadline:
                    raise

    def _rpc(self, method: str, params: dict[str, Any]) -> Any:
        assert self._f is not None
        self._id += 1
        my = self._id
        line = json.dumps({"jsonrpc": "2.0", "id": my, "method": method, "params": params}) + "\n"
        self._f.write(line.encode("utf-8"))
        while True:
            while b"\n" not in self._buf:
                chunk = self._f.read(65536)
                if not chunk:
                    self.close()
                    raise XtraOSError("NOT_PAIRED", "connection to the XtraOS core closed")
                self._buf += chunk
            raw, self._buf = self._buf.split(b"\n", 1)
            msg = json.loads(raw)
            if msg.get("id") != my:
                continue
            if "error" in msg:
                e = msg["error"] or {}
                raise XtraOSError(e.get("code", "INTERNAL"), e.get("message", "error"), e.get("hint", ""))
            return msg.get("result")

    # -- public ---------------------------------------------------------------------------
    def call(self, tool: str, **params: Any) -> Any:
        """Calls one XtraOS tool (same names/arguments as the MCP tools, docs/04)."""
        self._connect()
        return self._rpc(tool, params)

    def close(self) -> None:
        if self._f is not None:
            try:
                self._f.close()
            finally:
                self._f = None

    def __enter__(self) -> "XtraOS":
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()
