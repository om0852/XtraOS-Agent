# ADR 0073 — Team policies: a signed policy from the organisation, enforced as a floor

- Status: accepted (owner: "implement it", 2026-11-11; plan row P8-M4). The organisation model
  is chosen here (simple, changeable later): one admin key pair per organisation.
- Date: 2026-11-11

## Context
A company deploying XtraOS wants rules that users and agents cannot loosen: e.g. never send
files, no WhatsApp, a given agent read-only. Today every rule lives in the user's own
`policy.json`, which the user (or a program running as them) can change.

## Decision
1. **Where**: `%ProgramData%\XtraOS\team-policy.json` (override for tests: `XTRAOS_TEAM_DIR`).
   An administrator writes it; standard users can read but not change files an administrator
   created there, so a user-level agent cannot edit or delete it.
2. **Signed**: the file is `{"policy": {…}, "key_id": "…", "signature": "…"}`. The signature is
   ECDSA P-256 over SHA-256 of the canonical JSON text of `policy` (Windows CNG, no extra
   dependency). Trusted public keys live in `%ProgramData%\XtraOS\team-keys\<key_id>.pub`.
   Tools: `xtraos-core --team-keygen <dir>` makes a key pair (the private key stays with the
   admin, never in ProgramData); `xtraos-core --team-sign <policy.json> --key <private> --out
   <team-policy.json>` signs.
3. **A floor, never a ceiling**: the team policy can only make things stricter. Per action the
   stricter of the user's and the team's decision wins (deny > confirm > allow), for classes
   and apps; per client the stricter profile (blocked > read_only > background_only > full);
   `rate_per_minute` and `max_actions_per_task` take the lower value. Only what the team file
   names applies; everything else is the user's policy.
4. **Fail closed**: a team policy file that is present but does not verify (bad signature,
   unknown key, unreadable) blocks every acting call with `BLOCKED_BY_POLICY` ("team policy could
   not be verified"); reading still works. Deleting the file needs administrator rights.
5. **Visible**: `status.team` says whether a team policy is active (key id) or broken; the rules
   viewer shows "set by your organisation"; the action log records the team policy's hash.

## Tests
- Unit: sign/verify round trip, a changed policy fails, unknown key fails; merge rules
  (stricter wins; unnamed entries untouched; numbers take the minimum).
- Interactive: a core with a team directory: a team `deny` beats a user `allow`; a team-blocked
  client is blocked; a tampered file blocks acting calls but not reading.
