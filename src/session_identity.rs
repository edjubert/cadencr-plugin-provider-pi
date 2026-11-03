//! Opaque ACP identities bound to the canonical workspace and exact native ID.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use std::path::{Path, PathBuf};

const PREFIX: &str = "pi-rust-v1.";

pub fn workspace(cwd: &str) -> Result<PathBuf, String> {
    let path = Path::new(cwd);
    if !path.is_absolute() {
        return Err("Pi session workspace must be an absolute path".into());
    }
    path.canonicalize()
        .map_err(|error| format!("cannot resolve Pi workspace: {error}"))
}

pub fn encode(cwd: &Path, native_id: &str) -> Result<String, String> {
    if !valid_native_id(native_id) {
        return Err("Pi returned an invalid native session UUID".into());
    }
    let payload = serde_json::to_vec(&(cwd, native_id)).map_err(|error| error.to_string())?;
    Ok(format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(payload)))
}

pub fn decode(session_id: &str, cwd: &Path) -> Result<String, String> {
    let payload = session_id.strip_prefix(PREFIX).ok_or(
        "Pi session ID is not workspace-bound; start a new conversation with this connector version",
    )?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| "invalid Pi session ID")?;
    let (stored_cwd, native_id): (PathBuf, String) =
        serde_json::from_slice(&bytes).map_err(|_| "invalid Pi session ID")?;
    if stored_cwd != cwd {
        return Err("Pi session belongs to another workspace".into());
    }
    if encode(cwd, &native_id)? != session_id {
        return Err("non-canonical Pi session ID".into());
    }
    Ok(native_id)
}

fn valid_native_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "12345678-1234-4234-8234-123456789abc";

    #[test]
    fn identity_round_trips_only_in_its_workspace() {
        let cwd = Path::new("/workspace");
        let id = encode(cwd, UUID).unwrap();
        assert_eq!(decode(&id, cwd).unwrap(), UUID);
        assert!(decode(&id, Path::new("/other")).is_err());
    }

    #[test]
    fn refuses_legacy_paths_partial_ids_and_malformed_payloads() {
        for id in [UUID, "/tmp/session.jsonl", "pi-rust-v1.invalid"] {
            assert!(decode(id, Path::new("/workspace")).is_err());
        }
        for native_id in ["", "12345678", "/tmp/session.jsonl"] {
            assert!(encode(Path::new("/workspace"), native_id).is_err());
        }
        assert!(workspace("relative").is_err());
    }
}
