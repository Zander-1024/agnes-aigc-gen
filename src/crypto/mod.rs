use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use sha2::{Digest, Sha256};

const NONCE_LEN: usize = 12;

/// Machine-bound fingerprint derived from OS identity at runtime (never stored in config).
pub fn machine_id() -> Result<String> {
    let platform = platform_machine_uuid()?;
    let input = format!("{}:{}:{}", platform, std::env::consts::OS, std::env::consts::ARCH);
    Ok(hex_hash(&input))
}

#[cfg(target_os = "macos")]
fn platform_machine_uuid() -> Result<String> {
    use std::process::Command;
    let output = Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .context("run ioreg for IOPlatformUUID")?;
    if !output.status.success() {
        bail!("ioreg failed with status {}", output.status);
    }
    let stdout = String::from_utf8(output.stdout).context("decode ioreg output")?;
    for line in stdout.lines() {
        if line.contains("IOPlatformUUID")
            && let Some(uuid) = line.split('"').nth(3)
            && !uuid.is_empty()
        {
            return Ok(uuid.to_string());
        }
    }
    bail!("IOPlatformUUID not found")
}

#[cfg(target_os = "linux")]
fn platform_machine_uuid() -> Result<String> {
    for path in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        if let Ok(raw) = std::fs::read_to_string(path) {
            let id = raw.trim();
            if !id.is_empty() {
                return Ok(id.to_string());
            }
        }
    }
    bail!("linux machine-id not found")
}

#[cfg(target_os = "windows")]
fn platform_machine_uuid() -> Result<String> {
    use std::process::Command;
    let output = Command::new("reg")
        .args(["query", r"HKLM\SOFTWARE\Microsoft\Cryptography", "/v", "MachineGuid"])
        .output()
        .context("query Windows MachineGuid")?;
    if !output.status.success() {
        bail!("reg query failed with status {}", output.status);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.starts_with("MachineGuid") {
            if let Some(guid) = line.split_whitespace().last() {
                if !guid.is_empty() {
                    return Ok(guid.to_string());
                }
            }
        }
    }
    bail!("MachineGuid not found")
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn platform_machine_uuid() -> Result<String> {
    bail!("machine-bound API key encryption is not supported on this platform")
}

fn derive_key() -> Result<[u8; 32]> {
    let machine_id = machine_id()?;
    let mut hasher = Sha256::new();
    hasher.update(b"agnes-aigc-gen:v1:");
    hasher.update(machine_id.as_bytes());
    Ok(hasher.finalize().into())
}

pub fn encrypt_api_key(plain: &str) -> Result<String> {
    let key = derive_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| anyhow::anyhow!("invalid cipher key"))?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    use rand::Rng;
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plain.as_bytes())
        .map_err(|_| anyhow::anyhow!("encryption failed"))?;
    let mut payload = nonce_bytes.to_vec();
    payload.extend(ciphertext);
    Ok(B64.encode(payload))
}

pub fn decrypt_api_key(encoded: &str) -> Result<String> {
    let payload = B64.decode(encoded).context("decode api_key_encrypted")?;
    if payload.len() <= NONCE_LEN {
        bail!("invalid encrypted api key payload");
    }
    let (nonce_bytes, ciphertext) = payload.split_at(NONCE_LEN);
    let key = derive_key()?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| anyhow::anyhow!("invalid cipher key"))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    let plain = cipher.decrypt(nonce, ciphertext).map_err(|_| {
        anyhow::anyhow!("decryption failed; re-run on this machine: agnes-aigc-gen config set api-key <KEY>")
    })?;
    String::from_utf8(plain).context("api key utf8")
}

fn hex_hash(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex_lower(hasher.finalize())
}

fn hex_lower(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_api_key() {
        let enc = encrypt_api_key("sk-test-key").unwrap();
        let dec = decrypt_api_key(&enc).unwrap();
        assert_eq!(dec, "sk-test-key");
    }

    #[test]
    fn platform_machine_uuid_available() {
        platform_machine_uuid().expect("platform machine uuid");
    }
}
