# xtraos (Python)

Call XtraOS Agent tools from Python scripts and agents. Windows only, standard library only.

```python
from xtraos import XtraOS, XtraOSError

with XtraOS() as x:                      # finds and pairs with your XtraOS core
    print(x.call("volume", op="get")["text"])
    windows = x.call("list_windows", filter="notepad")["windows"]
    try:
        x.call("system", op="lock")      # asks you first (approval dialog)
    except XtraOSError as e:
        print(e.code, e.hint)            # e.g. BLOCKED_BY_POLICY
```

- **Tools.** The tool names and arguments are the same as the MCP tools
  (`docs/04-mcp-tool-contract.md`).
- **Pairing.** It pairs with your own token, so your rules, approval dialogs, the kill
  switch (Ctrl+Alt+Esc) and the action log all apply.
- **Starting the core.** If the core is not running, the client starts the installed one.
  `XtraOS(autostart=False)` turns that off.
- **Options:** `XtraOS(pipe=..., token=..., client=..., autostart=...)`. The environment
  variables `XTRAOS_PIPE`, `XTRAOS_TOKEN` and `XTRAOS_HOME` override the defaults.
- **Install locally:** `pip install ./sdk/python`.
