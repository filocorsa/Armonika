//! Symmetric encryption of the secrets the server must be able to read back, such as
//! the Subsonic passwords (the Subsonic token authentication needs them in clear).
//!
//! The 32-byte key comes from `ARMONIKA_SECRET_KEY` (base64) when set, otherwise from
//! the file `secret.key` in the data directory, created on first start.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use crate::{Error, Result};

/// Name of the key file inside the data directory.
pub const KEY_FILE: &str = "secret.key";

/// Environment variable that holds the key in base64, overriding the key file.
pub const KEY_ENV: &str = "ARMONIKA_SECRET_KEY";

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;

/// Seals and opens small secrets with XChaCha20-Poly1305. Sealed data is the random
/// 24-byte nonce followed by the ciphertext and its tag.
#[derive(Clone)]
pub struct SecretBox {
    cipher: XChaCha20Poly1305,
}

impl std::fmt::Debug for SecretBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretBox(..)")
    }
}

impl SecretBox {
    /// A box using `key` directly.
    pub fn from_key(key: [u8; KEY_LEN]) -> SecretBox {
        SecretBox {
            cipher: XChaCha20Poly1305::new(&key.into()),
        }
    }

    /// Loads the key from `ARMONIKA_SECRET_KEY` if set and not empty, otherwise from
    /// `data_dir/secret.key`, creating that file with a new random key if missing.
    pub fn load_or_create(data_dir: &Path) -> Result<SecretBox> {
        let env = match std::env::var(KEY_ENV) {
            Ok(v) => Some(v),
            Err(std::env::VarError::NotPresent) => None,
            Err(std::env::VarError::NotUnicode(_)) => {
                return Err(Error::InvalidSecretKey {
                    from: KeySource::Env,
                });
            }
        };
        Self::load_or_create_with(data_dir, env.as_deref())
    }

    /// Like [`SecretBox::load_or_create`], with the value of `ARMONIKA_SECRET_KEY`
    /// given by the caller.
    ///
    /// A new key file is written in full to a temporary file first and then linked
    /// into place without replacing an existing file, so a crash or two servers
    /// starting together never leave an empty or partial `secret.key`; when another
    /// process wins the race, its key is used. An existing key file that is not valid
    /// is reported, never replaced, because data sealed with it would be lost.
    pub fn load_or_create_with(data_dir: &Path, env_key: Option<&str>) -> Result<SecretBox> {
        if let Some(b64) = env_key.filter(|v| !v.trim().is_empty()) {
            return Ok(SecretBox::from_key(decode_key(b64, KeySource::Env)?));
        }
        let path = data_dir.join(KEY_FILE);
        let file_error = |source| Error::SecretKeyFile {
            path: path.clone(),
            source,
        };
        if let Some(key) = read_key_file(&path)? {
            return Ok(SecretBox::from_key(key));
        }

        let mut key = [0; KEY_LEN];
        random_bytes(&mut key)?;
        let mut suffix = [0; 8];
        random_bytes(&mut suffix)?;
        let tmp = data_dir.join(format!(
            ".{KEY_FILE}.{}.tmp",
            suffix
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ));
        write_new_file(&tmp, &key).map_err(|e| Error::SecretKeyFile {
            path: tmp.clone(),
            source: e,
        })?;
        let mut linked = fs::hard_link(&tmp, &path);
        let _ = fs::remove_file(&tmp);
        if let Err(e) = &linked
            && e.kind() != ErrorKind::AlreadyExists
        {
            // The file system has no hard links (for example FAT): fall back to
            // creating the file in place, still never replacing an existing one.
            linked = write_new_file(&path, &key);
        }
        match linked {
            Ok(()) => Ok(SecretBox::from_key(key)),
            // Another process created it in the meantime: use theirs.
            Err(e) if e.kind() == ErrorKind::AlreadyExists => read_key_file(&path)?
                .map(SecretBox::from_key)
                .ok_or_else(|| file_error(e)),
            Err(e) => Err(file_error(e)),
        }
    }

    /// Encrypts `plain` with a fresh random nonce.
    pub fn seal(&self, plain: &[u8]) -> Vec<u8> {
        let mut nonce = [0; NONCE_LEN];
        // Without a working system random generator nothing secure can be done.
        random_bytes(&mut nonce).expect("system random number generator failed");
        let ciphertext = self
            .cipher
            .encrypt(&XNonce::from(nonce), plain)
            .expect("XChaCha20-Poly1305 encryption cannot fail for in-memory data");
        let mut sealed = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&ciphertext);
        sealed
    }

    /// Decrypts data produced by [`SecretBox::seal`] with the same key.
    pub fn open(&self, sealed: &[u8]) -> Result<Vec<u8>> {
        if sealed.len() < NONCE_LEN {
            return Err(Error::Decrypt);
        }
        let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
        let nonce = XNonce::try_from(nonce).map_err(|_| Error::Decrypt)?;
        self.cipher
            .decrypt(&nonce, ciphertext)
            .map_err(|_| Error::Decrypt)
    }
}

/// Where a secret key came from, for error messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeySource {
    /// The `ARMONIKA_SECRET_KEY` variable.
    Env,
    /// The key file at this path.
    File(PathBuf),
}

impl std::fmt::Display for KeySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeySource::Env => write!(f, "the {KEY_ENV} variable"),
            KeySource::File(path) => write!(f, "the file {}", path.display()),
        }
    }
}

/// The key in the file at `path`, or `None` if the file does not exist.
fn read_key_file(path: &Path) -> Result<Option<[u8; KEY_LEN]>> {
    match fs::read_to_string(path) {
        Ok(text) => decode_key(&text, KeySource::File(path.to_path_buf())).map(Some),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) if e.kind() == ErrorKind::InvalidData => Err(Error::InvalidSecretKey {
            from: KeySource::File(path.to_path_buf()),
        }),
        Err(source) => Err(Error::SecretKeyFile {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Writes `key` in base64 to the new file `path`, readable only by the owner on Unix,
/// and flushes it to disk.
fn write_new_file(path: &Path, key: &[u8; KEY_LEN]) -> std::io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    writeln!(file, "{}", STANDARD.encode(key))?;
    file.sync_all()
}

fn decode_key(b64: &str, from: KeySource) -> Result<[u8; KEY_LEN]> {
    STANDARD
        .decode(b64.trim())
        .ok()
        .and_then(|bytes| <[u8; KEY_LEN]>::try_from(bytes).ok())
        .ok_or(Error::InvalidSecretKey { from })
}

/// Fills `buf` from the operating system random number generator.
pub(crate) fn random_bytes(buf: &mut [u8]) -> Result<()> {
    getrandom::fill(buf).map_err(|e| Error::Random(e.to_string()))
}
