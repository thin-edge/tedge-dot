//! Shared helpers for the OPC UA security tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The repository root: the closest ancestor of the manifest directory holding genpki.py
/// (this module is shared by several crates' tests).
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|dir| dir.join(GENPKI).exists())
        .expect("repository root")
        .to_path_buf()
}

const GENPKI: &str = "connectors/opcua/conformance/tools/genpki.py";

/// A fresh, empty directory under the system temp dir.
pub fn tempdir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "tedge-dot-opcua-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The Python that runs `genpki.py`: `$TEDGE_DOT_PYTHON`, the repository's `.venv`, or
/// `python3`. It needs the `cryptography` package (`just venv`, or
/// `pip install cryptography`). Missing it fails the test: these tests must not pass by
/// generating nothing.
fn python() -> PathBuf {
    if let Some(p) = std::env::var_os("TEDGE_DOT_PYTHON") {
        return PathBuf::from(p);
    }
    let venv = repo_root().join(".venv/bin/python");
    if venv.exists() {
        return venv;
    }
    PathBuf::from("python3")
}

/// Generate the shared PKI test vectors (connectors/opcua/conformance/tools/genpki.py) into a
/// fresh directory, with valid server certificates naming `hostnames`.
pub fn genpki(hostnames: &[&str]) -> PathBuf {
    let out = tempdir("pki-vectors");
    let script = repo_root().join(GENPKI);
    let mut cmd = Command::new(python());
    cmd.arg(&script).arg(&out);
    for h in hostnames {
        cmd.arg("--hostname").arg(h);
    }
    let result = cmd.output().unwrap_or_else(|e| {
        panic!(
            "cannot run {} (set TEDGE_DOT_PYTHON): {e}",
            python().display()
        )
    });
    assert!(
        result.status.success(),
        "genpki.py failed (does the Python have `cryptography`? run `just venv`):\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
    out
}

/// `expected.json` of a vector tree.
pub fn expected(vectors: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(vectors.join("expected.json")).unwrap()).unwrap()
}
