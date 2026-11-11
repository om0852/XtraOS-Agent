# xtraos-install

Installs [XtraOS Agent](https://github.com/om0852/XtraOS-Agent) for the current Windows user:

```
npx xtraos-install
```

It downloads the release zip and `SHA256SUMS.txt` from the project's GitHub Releases only,
refuses the zip if its SHA-256 does not match, extracts it and runs its `install.ps1` (no
administrator rights needed). Options: `--version X.Y.Z`, `--from <zip>` (a local zip with
`SHA256SUMS.txt` next to it), `--dry-run` (verify only).
