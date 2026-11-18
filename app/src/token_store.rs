use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use keyring::{Entry, Error as KeyringError};
use zeroize::Zeroizing;

const SERVICE: &str = "app.musicpresence.discord";
const TOKEN_USER: &str = "bot-token-ciphertext";
const KEY_USER: &str = "bot-token-encryption-key";
const VERSION: u8 = 1;
const NONCE_LENGTH: usize = 12;
const AAD: &[u8] = b"music-presence:discord-bot-token:v1";

pub fn save(token: &str) -> Result<(), String> {
    let key_entry = entry(KEY_USER)?;
    let (key, created_key) = match key_entry.get_secret() {
        Ok(key) => (Zeroizing::new(key), false),
        Err(KeyringError::NoEntry) => {
            let mut key = Zeroizing::new(vec![0; 32]);
            getrandom::fill(&mut key)
                .map_err(|error| format!("Could not generate encryption key: {error}"))?;
            key_entry.set_secret(&key).map_err(|error| {
                format!("Could not store encryption key in OS credential store: {error}")
            })?;
            (key, true)
        }
        Err(error) => {
            return Err(format!(
                "Could not read encryption key from OS credential store: {error}"
            ));
        }
    };

    let encrypted = encrypt(&key, token.as_bytes())?;
    let token_entry = entry(TOKEN_USER)?;
    if let Err(error) = token_entry.set_secret(&encrypted) {
        if created_key {
            delete_if_present(&key_entry)?;
        }
        return Err(format!("Could not store encrypted bot token: {error}"));
    }

    Ok(())
}

pub fn load() -> Result<Option<Zeroizing<String>>, String> {
    let token_entry = entry(TOKEN_USER)?;
    let encrypted = match token_entry.get_secret() {
        Ok(encrypted) => encrypted,
        Err(KeyringError::NoEntry) => return Ok(None),
        Err(error) => return Err(format!("Could not read encrypted bot token: {error}")),
    };

    let key_entry = entry(KEY_USER)?;
    let key = key_entry
        .get_secret()
        .map(Zeroizing::new)
        .map_err(|error| {
            format!("Could not read encryption key from OS credential store: {error}")
        })?;
    let plaintext = decrypt(&key, &encrypted)?;
    let token = String::from_utf8(plaintext)
        .map_err(|_| "Stored bot token is not valid UTF-8".to_string())?;

    Ok(Some(Zeroizing::new(token)))
}

pub fn clear() -> Result<(), String> {
    let token_entry = entry(TOKEN_USER)?;
    let key_entry = entry(KEY_USER)?;
    let token_result = delete_if_present(&token_entry);
    let key_result = delete_if_present(&key_entry);

    match (token_result, key_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(token_error), Ok(())) => Err(token_error),
        (Ok(()), Err(key_error)) => Err(key_error),
        (Err(token_error), Err(key_error)) => Err(format!("{token_error}; {key_error}")),
    }
}

fn entry(user: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, user).map_err(|error| {
        format!("OS credential store is unavailable; bot token was not stored: {error}")
    })
}

fn delete_if_present(entry: &Entry) -> Result<(), String> {
    match entry.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(error) => Err(format!("Could not remove a saved credential: {error}")),
    }
}

fn encrypt(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| "Stored encryption key has an invalid length".to_string())?;
    let mut nonce_bytes = [0; NONCE_LENGTH];
    getrandom::fill(&mut nonce_bytes)
        .map_err(|error| format!("Could not generate encryption nonce: {error}"))?;
    let nonce = Nonce::try_from(&nonce_bytes[..])
        .map_err(|_| "Could not create encryption nonce".to_string())?;
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad: AAD,
            },
        )
        .map_err(|_| "Could not encrypt bot token".to_string())?;

    let mut envelope = Vec::with_capacity(1 + NONCE_LENGTH + ciphertext.len());
    envelope.push(VERSION);
    envelope.extend_from_slice(nonce.as_slice());
    envelope.extend_from_slice(&ciphertext);
    Ok(envelope)
}

fn decrypt(key: &[u8], envelope: &[u8]) -> Result<Vec<u8>, String> {
    if envelope.len() <= 1 + NONCE_LENGTH || envelope[0] != VERSION {
        return Err("Saved bot token has an unsupported or invalid format".into());
    }

    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| "Stored encryption key has an invalid length".to_string())?;
    let nonce = Nonce::try_from(&envelope[1..1 + NONCE_LENGTH])
        .map_err(|_| "Saved bot token has an invalid nonce".to_string())?;
    cipher
        .decrypt(
            &nonce,
            Payload {
                msg: &envelope[1 + NONCE_LENGTH..],
                aad: AAD,
            },
        )
        .map_err(|_| "Saved bot token failed authentication".to_string())
}

#[cfg(test)]
mod tests {
    use super::{decrypt, encrypt};

    #[test]
    fn encrypts_and_authenticates_token() {
        let mut key = vec![0; 32];
        getrandom::fill(&mut key).expect("system random source should be available");
        let plaintext = b"bot-token-test";
        let mut envelope = encrypt(&key, plaintext).expect("encryption should succeed");

        assert_ne!(&envelope[13..], plaintext);
        assert_eq!(
            decrypt(&key, &envelope).expect("decryption should succeed"),
            plaintext
        );

        let last = envelope.len() - 1;
        envelope[last] ^= 1;
        assert!(decrypt(&key, &envelope).is_err());
    }
}
