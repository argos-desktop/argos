use keyring::Entry;
use std::{collections::HashMap, sync::Mutex};

const SERVICE: &str = "app.argos.desktop";

/// Provider keys held by the backend. A key never travels back to the UI.
#[derive(Default)]
pub struct Keys(Mutex<HashMap<&'static str, (String, bool)>>);

impl Keys {
    /// The key and whether it is saved in the OS credential store.
    pub fn get(&self, account: &'static str) -> Option<(String, bool)> {
        let mut keys = self.0.lock().unwrap();
        if let Some(found) = keys.get(account) {
            return Some(found.clone());
        }
        let key = Entry::new(SERVICE, account)
            .and_then(|e| e.get_password())
            .ok()?;
        keys.insert(account, (key.clone(), true));
        Some((key, true))
    }

    /// Returns false when the key is kept for this session only.
    pub fn set(&self, account: &'static str, key: String, remember: bool) -> bool {
        let entry = Entry::new(SERVICE, account);
        let remembered = remember && entry.and_then(|e| e.set_password(&key)).is_ok();
        if !remembered {
            // An older saved key must not come back after a restart.
            let _ = Entry::new(SERVICE, account).and_then(|e| e.delete_credential());
        }
        self.0.lock().unwrap().insert(account, (key, remembered));
        remembered
    }

    pub fn forget(&self, account: &'static str) {
        self.0.lock().unwrap().remove(account);
        let _ = Entry::new(SERVICE, account).and_then(|e| e.delete_credential());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_only_key_is_held_in_memory_and_forgotten() {
        keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        let keys = Keys::default();
        assert!(keys.get("anthropic").is_none());
        assert!(!keys.set("anthropic", "secret".into(), false));
        assert_eq!(keys.get("anthropic"), Some(("secret".into(), false)));
        assert!(keys.get("openai").is_none());
        keys.forget("anthropic");
        assert!(keys.get("anthropic").is_none());
    }
}
