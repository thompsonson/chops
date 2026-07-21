use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use rand::RngCore;
use ssh_key::private::{Ed25519Keypair, PrivateKey};
use std::path::Path;

const KEY_ALIAS: &str = "chops-ssh-key";
const NONCE_LEN: usize = 12;

// ---------------------------------------------------------------------------
// SSH key generation
// ---------------------------------------------------------------------------

/// Generate a new ED25519 keypair, returning (private_key_bytes, public_key_openssh)
pub fn generate_ssh_key() -> Result<(Vec<u8>, String), String> {
    let kp = Ed25519Keypair::random(&mut OsRng);
    let private_key = PrivateKey::from(kp);
    let private_pem = private_key
        .to_openssh(ssh_key::LineEnding::LF)
        .map_err(|e| format!("Failed to serialize private key: {e}"))?;
    let public_openssh = private_key
        .public_key()
        .to_openssh()
        .map_err(|e| format!("Failed to serialize public key: {e}"))?;
    Ok((private_pem.as_bytes().to_vec(), public_openssh))
}

// ---------------------------------------------------------------------------
// AES-256-GCM encryption / decryption
// ---------------------------------------------------------------------------

/// Generate a new random AES-256 key
fn generate_aes_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    key
}

/// Load or create the AES encryption key from app data dir.
fn get_or_create_aes_key(app_data: &Path) -> Result<[u8; 32], String> {
    let key_path = app_data.join("chops-key.bin");
    if let Ok(existing) = std::fs::read(&key_path) {
        if existing.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&existing);
            return Ok(key);
        }
    }
    let key = generate_aes_key();
    std::fs::create_dir_all(app_data).map_err(|e| format!("Cannot create app data dir: {e}"))?;
    std::fs::write(&key_path, &key).map_err(|e| format!("Cannot write key file: {e}"))?;
    Ok(key)
}

/// Encrypt plaintext with AES-256-GCM. Returns nonce || ciphertext.
pub fn encrypt_blob(app_data: &Path, plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let key_bytes = get_or_create_aes_key(app_data)?;
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| format!("Encryption failed: {e}"))?;

    // Prepend nonce to ciphertext
    let mut result = nonce_bytes.to_vec();
    result.extend(ciphertext);
    Ok(result)
}

/// Decrypt nonce || ciphertext with AES-256-GCM.
pub fn decrypt_blob(app_data: &Path, encrypted: &[u8]) -> Result<Vec<u8>, String> {
    if encrypted.len() < NONCE_LEN {
        return Err("Invalid encrypted blob".into());
    }

    let key_bytes = get_or_create_aes_key(app_data)?;
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);

    let (nonce_bytes, ciphertext) = encrypted.split_at(NONCE_LEN);
    let nonce = Nonce::from_slice(nonce_bytes);

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| "Decryption failed (wrong key or corrupted)".into())
}

// ---------------------------------------------------------------------------
// Key storage (encrypted on disk, key in app_data)
// ---------------------------------------------------------------------------

/// Store an SSH private key encrypted to disk.
pub fn store_ssh_key(app_data: &Path, alias: &str, private_key: &[u8]) -> Result<(), String> {
    let encrypted = encrypt_blob(app_data, private_key)?;
    let path = app_data.join(format!("{KEY_ALIAS}-{alias}.enc"));
    std::fs::write(&path, &encrypted).map_err(|e| format!("Cannot write encrypted key: {e}"))
}

/// Load and decrypt an SSH private key.
pub fn load_ssh_key(app_data: &Path, alias: &str) -> Result<Vec<u8>, String> {
    let path = app_data.join(format!("{KEY_ALIAS}-{alias}.enc"));
    let encrypted =
        std::fs::read(&path).map_err(|e| format!("Cannot read encrypted key for {alias}: {e}"))?;
    decrypt_blob(app_data, &encrypted)
}

/// Check if an encrypted key exists.
pub fn has_ssh_key(app_data: &Path, alias: &str) -> bool {
    let path = app_data.join(format!("{KEY_ALIAS}-{alias}.enc"));
    path.exists()
}

/// Delete an encrypted key.
pub fn delete_ssh_key(app_data: &Path, alias: &str) {
    let path = app_data.join(format!("{KEY_ALIAS}-{alias}.enc"));
    let _ = std::fs::remove_file(&path);
}

/// Load the public key in OpenSSH format from the stored private key.
pub fn load_ssh_public_key(app_data: &Path, alias: &str) -> Result<String, String> {
    let private_bytes = load_ssh_key(app_data, alias)?;
    let pem = std::str::from_utf8(&private_bytes)
        .map_err(|_| "Invalid UTF-8 in private key".to_string())?;
    let private_key = ssh_key::PrivateKey::from_openssh(pem)
        .map_err(|e| format!("Failed to parse private key: {e}"))?;
    private_key
        .public_key()
        .to_openssh()
        .map_err(|e| format!("Failed to serialize public key: {e}"))
}

/// Store the SSH username for a host alias.
pub fn store_ssh_username(app_data: &Path, alias: &str, username: &str) -> Result<(), String> {
    let path = app_data.join(format!("{KEY_ALIAS}-{alias}.user"));
    std::fs::write(&path, username).map_err(|e| format!("Cannot write username: {e}"))
}

/// Load the SSH username for a host alias. Returns "mt" if not found.
pub fn load_ssh_username(app_data: &Path, alias: &str) -> String {
    let path = app_data.join(format!("{KEY_ALIAS}-{alias}.user"));
    std::fs::read_to_string(&path).unwrap_or_else(|_| "mt".to_string())
}

/// Store the SSH port for a host alias.
pub fn store_ssh_port(app_data: &Path, alias: &str, port: u16) -> Result<(), String> {
    let path = app_data.join(format!("{KEY_ALIAS}-{alias}.port"));
    std::fs::write(&path, port.to_string()).map_err(|e| format!("Cannot write port: {e}"))
}

/// Load the SSH port for a host alias. Returns 22 if not found.
pub fn load_ssh_port(app_data: &Path, alias: &str) -> u16 {
    let path = app_data.join(format!("{KEY_ALIAS}-{alias}.port"));
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(22)
}
