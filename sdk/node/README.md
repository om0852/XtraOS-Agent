# @xtraos/sdk (Node)

Call XtraOS Agent tools from Node scripts and agents. Windows only, no dependencies.

```js
import { XtraOS, XtraOSError } from '@xtraos/sdk';

const x = new XtraOS();                       // finds and pairs with your XtraOS core
console.log((await x.call('volume', { op: 'get' })).text);
const w = await x.call('list_windows', { filter: 'notepad' });
try {
  await x.call('system', { op: 'lock' });    // asks you first (approval dialog)
} catch (e) {
  if (e instanceof XtraOSError) console.log(e.code, e.hint); // e.g. BLOCKED_BY_POLICY
}
x.close();
```

- **Tools.** The tool names and arguments are the same as the MCP tools
  (`docs/04-mcp-tool-contract.md`).
- **Pairing.** It pairs with your own token, so your rules, approval dialogs, the kill
  switch (Ctrl+Alt+Esc) and the action log all apply.
- **Starting the core.** If the core is not running, the SDK starts the installed one
  (`%LOCALAPPDATA%\Programs\XtraOS`). `autostart: false` turns that off.
- **Options:** `{ pipe, token, client, autostart, timeoutMs }`. The environment variables
  `XTRAOS_PIPE`, `XTRAOS_TOKEN` and `XTRAOS_HOME` override the defaults.
