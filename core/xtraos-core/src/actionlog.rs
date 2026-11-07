//! Append-only action log (JSONL). One line per handled request. Never contains request
//! params or the pairing token (CLAUDE.md §5 logging rule).
//!
//! Tamper evidence (ADR 0038): every line carries `seq`, `prev` (the previous line's hash) and
//! `hash` = SHA-256 of the line's canonical JSON without `hash` (serde_json, sorted keys). A
//! new file continues the chain from the newest existing file, so edits, deletions,
//! reordering or a removed older file break it; `verify_dir` finds the first break. Honest
//! limit: without a secret, software running as the same user could rewrite the whole chain.
use serde::Serialize;
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Serialize)]
pub struct Entry<'a> {
    pub ts_ms: u128,
    pub session: Option<&'a str>,
    pub client: Option<&'a str>,
    pub method: &'a str,
    pub ok: bool,
    pub code: Option<&'a str>,
    pub ms: f64,
    /// Policy decision for OS actions (M5): allow / confirmed / blocked; or a policy reload note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<&'a str>,
}

struct Chain {
    file: File,
    seq: u64,
    last: String,
}

pub struct ActionLog {
    path: PathBuf,
    state: Mutex<Chain>,
}

// ---------------------------------------------------------------------------------------------
// SHA-256 (FIPS 180-4), small and dependency-free

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[4 * i],
                block[4 * i + 1],
                block[4 * i + 2],
                block[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
}

/// Hash of a log object (without its `hash` field), canonical serde_json form.
fn line_hash(v: &Value) -> String {
    let mut o = v.clone();
    if let Some(m) = o.as_object_mut() {
        m.remove("hash");
    }
    sha256_hex(serde_json::to_string(&o).unwrap_or_default().as_bytes())
}

// ---------------------------------------------------------------------------------------------
// Files

/// `actions-<unix-ms>-<pid>.jsonl` files of a log folder, oldest first.
pub fn log_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<(u64, String, PathBuf)> = fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .filter_map(|e| {
                    let p = e.path();
                    let n = p.file_name()?.to_str()?.to_string();
                    if !(n.starts_with("actions-") && n.ends_with(".jsonl")) {
                        return None;
                    }
                    let t = n.split('-').nth(1)?.parse::<u64>().ok()?;
                    Some((t, n, p))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files.into_iter().map(|f| f.2).collect()
}

/// Last chained (seq, hash) in a file, if any.
fn last_link(path: &Path) -> Option<(u64, String)> {
    let t = fs::read_to_string(path).ok()?;
    t.lines().rev().find_map(|l| {
        let v: Value = serde_json::from_str(l).ok()?;
        Some((v["seq"].as_u64()?, v["hash"].as_str()?.to_string()))
    })
}

impl ActionLog {
    /// Creates `<dir>/actions-<unix-ms>-<pid>.jsonl`, continuing the hash chain of the newest
    /// existing file.
    pub fn create(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let (seq, last) = log_files(dir)
            .iter()
            .rev()
            .find_map(|p| last_link(p))
            .unwrap_or((0, GENESIS.to_string()));
        let path = dir.join(format!(
            "actions-{}-{}.jsonl",
            crate::protocol::now_ms(),
            std::process::id()
        ));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            state: Mutex::new(Chain { file, seq, last }),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn record(&self, entry: &Entry) {
        // Logging must never take the core down; failures go to stderr.
        let mut v = match serde_json::to_value(entry) {
            Ok(v) => v,
            Err(e) => return eprintln!("[xtraos-core] log serialise failed: {e}"),
        };
        let mut c = match self.state.lock() {
            Ok(c) => c,
            Err(p) => p.into_inner(),
        };
        let seq = c.seq + 1;
        if let Some(m) = v.as_object_mut() {
            m.insert("seq".into(), Value::from(seq));
            m.insert("prev".into(), Value::from(c.last.clone()));
        }
        let hash = line_hash(&v);
        if let Some(m) = v.as_object_mut() {
            m.insert("hash".into(), Value::from(hash.clone()));
        }
        let mut line = serde_json::to_vec(&v).unwrap_or_default();
        line.push(b'\n');
        match c.file.write_all(&line) {
            Ok(()) => {
                c.seq = seq;
                c.last = hash;
            }
            Err(e) => eprintln!("[xtraos-core] log write failed: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Verify

#[derive(Debug, PartialEq)]
pub struct Verify {
    pub ok: bool,
    pub entries: u64,
    pub files: usize,
    pub legacy: u64,
    pub message: String,
}

/// Checks the whole chain of a log folder; stops at the first break.
pub fn verify_dir(dir: &Path) -> Verify {
    let files = log_files(dir);
    let mut expected_prev = GENESIS.to_string();
    let mut expected_seq = 1u64;
    let (mut entries, mut legacy, mut started) = (0u64, 0u64, false);
    let fail = |msg: String, entries, legacy, n| Verify {
        ok: false,
        entries,
        files: n,
        legacy,
        message: msg,
    };
    for f in &files {
        let name = f
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let Ok(t) = fs::read_to_string(f) else {
            return fail(
                format!("{name}: cannot be read"),
                entries,
                legacy,
                files.len(),
            );
        };
        for (i, l) in t.lines().enumerate() {
            if l.trim().is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(l) else {
                return fail(
                    format!("{name} line {}: not valid JSON (edited?)", i + 1),
                    entries,
                    legacy,
                    files.len(),
                );
            };
            let Some(hash) = v["hash"].as_str() else {
                if started {
                    return fail(
                        format!(
                            "{name} line {}: an unchained line inside the chain (inserted?)",
                            i + 1
                        ),
                        entries,
                        legacy,
                        files.len(),
                    );
                }
                legacy += 1;
                continue;
            };
            started = true;
            let prev = v["prev"].as_str().unwrap_or("");
            if prev != expected_prev {
                let why = if i == 0 || entries == 0 {
                    "does not follow the previous file (a log file was removed or edited)"
                } else {
                    "does not follow the line before it (lines removed or reordered)"
                };
                return fail(
                    format!("{name} line {}: {why}", i + 1),
                    entries,
                    legacy,
                    files.len(),
                );
            }
            if v["seq"].as_u64() != Some(expected_seq) {
                return fail(format!("{name} line {}: sequence {} where {expected_seq} was expected (lines removed or inserted)", i + 1, v["seq"]), entries, legacy, files.len());
            }
            if line_hash(&v) != hash {
                return fail(
                    format!(
                        "{name} line {}: content changed after it was written",
                        i + 1
                    ),
                    entries,
                    legacy,
                    files.len(),
                );
            }
            expected_prev = hash.to_string();
            expected_seq += 1;
            entries += 1;
        }
    }
    Verify {
        ok: true,
        entries,
        files: files.len(),
        legacy,
        message: format!(
            "OK: {entries} entries in {} file(s) form an unbroken chain{}",
            files.len(),
            if legacy > 0 {
                format!(" ({legacy} older unchained lines before it)")
            } else {
                String::new()
            }
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            sha256_hex(&vec![b'a'; 1_000_000]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    fn entry(method: &str) -> Entry<'_> {
        Entry {
            ts_ms: 1,
            session: Some("s1"),
            client: Some("t"),
            method,
            ok: true,
            code: None,
            ms: 1.25,
            policy: None,
        }
    }

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("xtraos-log-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_two_files(d: &Path) -> (PathBuf, PathBuf) {
        let a = ActionLog::create(d).unwrap();
        a.record(&entry("one"));
        a.record(&entry("two"));
        let pa = a.path().to_path_buf();
        drop(a);
        std::thread::sleep(std::time::Duration::from_millis(5));
        let b = ActionLog::create(d).unwrap();
        b.record(&entry("three"));
        b.record(&entry("four"));
        (pa, b.path().to_path_buf())
    }

    #[test]
    fn chain_ok_across_files() {
        let d = dir("ok");
        write_two_files(&d);
        let v = verify_dir(&d);
        assert!(v.ok, "{v:?}");
        assert_eq!((v.entries, v.files), (4, 2));
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn edits_deletions_reorders_and_removed_files_are_found() {
        // edited content
        let d = dir("edit");
        let (a, _) = write_two_files(&d);
        let t = fs::read_to_string(&a)
            .unwrap()
            .replace("\"two\"", "\"TWO\"");
        fs::write(&a, t).unwrap();
        let v = verify_dir(&d);
        assert!(!v.ok && v.message.contains("content changed"), "{v:?}");
        // deleted middle line
        let d = dir("del");
        let (_, b) = write_two_files(&d);
        let t: Vec<String> = fs::read_to_string(&b)
            .unwrap()
            .lines()
            .map(String::from)
            .collect();
        fs::write(&b, format!("{}\n", t[1])).unwrap();
        let v = verify_dir(&d);
        assert!(!v.ok, "{v:?}");
        // reordered
        let d = dir("order");
        let (a, _) = write_two_files(&d);
        let t: Vec<String> = fs::read_to_string(&a)
            .unwrap()
            .lines()
            .map(String::from)
            .collect();
        fs::write(&a, format!("{}\n{}\n", t[1], t[0])).unwrap();
        assert!(!verify_dir(&d).ok);
        // an older file removed
        let d = dir("gone");
        let (a, _) = write_two_files(&d);
        fs::remove_file(&a).unwrap();
        let v = verify_dir(&d);
        assert!(!v.ok && v.message.contains("previous file"), "{v:?}");
        // legacy lines before the chain are accepted and counted
        let d = dir("legacy");
        fs::write(
            d.join("actions-1-1.jsonl"),
            "{\"ts_ms\":1,\"method\":\"old\",\"ok\":true}\n",
        )
        .unwrap();
        let a = ActionLog::create(&d).unwrap();
        a.record(&entry("new"));
        let v = verify_dir(&d);
        assert!(v.ok && v.legacy == 1 && v.entries == 1, "{v:?}");
        for t in ["edit", "del", "order", "gone", "legacy"] {
            let _ = fs::remove_dir_all(
                std::env::temp_dir().join(format!("xtraos-log-{t}-{}", std::process::id())),
            );
        }
    }
}
