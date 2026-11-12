# ADR 0076 — Answering XtraOS dialogs from a paired device (phone)

- Status: accepted (owner, 2026-11-12: "send all the approvals to the phone or any device from
  which I am accessing it")
- Date: 2026-11-12

## Context
Every XtraOS dialog (approvals for sending, deleting, settings; long-task and keyboard-borrow
questions; agent questions from `ask_human`) appears only on the PC screen and is answered with a
real click or key. When the owner is away from the PC, a step that needs approval waits 60 s and
fails. The owner drives the PC from a phone through Magnus AI (a separate project) and wants to
answer these dialogs there.

Today's rule is that only the human can answer, and XtraOS enforces it: its windows ignore input
injected by programs (ADR 0036/0041). A second way to answer must keep that rule: no program on
the PC, including an AI agent with the pairing token, may be able to answer.

## Decision
1. **Approver devices.** A device that may answer dialogs has an ECDSA P-256 key pair it made
   itself (in the phone's browser: WebCrypto, private key not extractable). XtraOS stores only
   public keys, in `approvers.json` in its data folder.
2. **Adding a device needs the human at the PC.** Core method `approvers.add {name, public_key}`
   (raw uncompressed P-256 key, base64) opens an XtraOS dialog on the PC: "Let the device "<name>"
   answer XtraOS dialogs from anywhere? Its code: XXXX-XXXX" (the first 8 hex characters of the
   key's SHA-256, which the device shows too). Only a real click on Allow adds it. One add at a
   time; 120 s. `approvers.list` and `approvers.remove {id}` (removing only reduces trust).
3. **Every open dialog gets a random id** (`uid`, 128 bits) when it opens.
4. **`approvals.list`** returns the open dialogs: uid, title, message, buttons, which button
   cancels, age.
5. **`approvals.answer {uid, button, device, issued_ms, signature}`.** XtraOS checks, in order:
   the device is an approver; `issued_ms` is within 2 minutes of now; a dialog with that uid is
   open; the button exists; the signature (P-256, SHA-256, r‖s) is valid over the exact text
   `xtraos-approval-v1\n<device>\n<uid>\n<button>\n<issued_ms>`. Only then is the dialog
   answered as if clicked, and closed. The action log records the `approvals.answer` call; the
   tray activity line and the reply name the device.
6. These are core methods (named pipe, pairing token), not MCP tools: the MCP server does not
   list them. An agent that calls them directly can list dialogs (they hold no secrets) and can
   ask to add a device (the human sees the dialog and can deny it), but cannot answer a dialog:
   that needs a device's private key. The policy does not apply to these methods (like
   `ask_human`); the kill switch does.
7. The PC dialog stays open while the device may answer; whichever answer comes first wins.

## Consequences
- The weakest link is now the phone: whoever holds an unlocked paired phone can approve. Remove
  a lost device with `approvers.remove` (Magnus: "remove device").
- A device must reach XtraOS through something on the PC (Magnus AI relays); XtraOS itself still
  listens only on its named pipe (CLAUDE.md §4).
- The 60 s approval timeout is unchanged; the owner has to answer within it on the phone.

## Tests
Unit: the signed text, key blob conversion, freshness window, wrong device, wrong uid, wrong
button, bad signature, a reused signature for another dialog, add/remove of approvers. A signature
made with the CNG key-pair code (as the phone would with WebCrypto) is accepted end to end on an
open dialog. Live: through Magnus, a real approval answered from the browser.
