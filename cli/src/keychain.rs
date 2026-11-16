//! The Apple keychain as a `config::Vault`: where the apps keep a
//! profile's DEKs and device token (`spec/cli.md` §Local state).
//!
//! One generic-password item per profile in the data protection
//! keychain (the only keychain on iOS; on macOS the one scoped by the
//! app's signature rather than by a per-item ACL, so it never prompts).
//! The item is `AfterFirstUnlockThisDeviceOnly` and not synchronizable:
//! readable by a background sync once the device has been unlocked
//! since boot, absent from backups and from iCloud Keychain, and never
//! restored onto another device. No access group is named, so it lands
//! in the app's default one (its application identifier) and no
//! extension can read it.
//!
//! That keychain needs an application identifier, which only a build
//! signed with a provisioning profile has. Without one writes fail
//! with `errSecMissingEntitlement`; `available` asks once and
//! `config::use_keychain` leaves the process on `secrets.toml` then.

use security_framework::access_control::{ProtectionMode, SecAccessControl};
use security_framework::base::Error;
use security_framework::passwords::{
    PasswordOptions, delete_generic_password_options, generic_password,
    set_generic_password_options,
};

use crate::config::{ConfigError, Vault};

const SERVICE: &str = "app.monoplan.profile";

// Security.framework `OSStatus` (SecBase.h).
// `errSecMissingEntitlement` is -34018.
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

pub struct Keychain;

/// The query that names a profile's item, for every call but the add.
fn item(id: &str) -> PasswordOptions {
    let mut options = PasswordOptions::new_generic_password(SERVICE, id);
    options.use_protected_keychain();
    options.set_access_synchronized(Some(false));
    options
}

fn failed(e: Error) -> ConfigError {
    ConfigError::Keychain(format!("{e} (OSStatus {})", e.code()))
}

/// Can this process use the data protection keychain? Asked by
/// deleting an item that does not exist, because a read does not tell:
/// on macOS an unentitled process's read is answered "no such item"
/// (measured from a cargo test binary, macOS 27.2), exactly what an
/// entitled one hears, and only a write or delete is refused with
/// `errSecMissingEntitlement`. `Profile::orphaned` discards a profile
/// on "no such item", so a process that fails this must never be
/// handed the keychain as its vault.
pub fn available() -> bool {
    match delete_generic_password_options(item("availability-probe")) {
        Ok(()) => true,
        Err(e) => e.code() == ERR_SEC_ITEM_NOT_FOUND,
    }
}

impl Vault for Keychain {
    fn get(&self, id: &str) -> Result<Option<Vec<u8>>, ConfigError> {
        match generic_password(item(id)) {
            Ok(secret) => Ok(Some(secret)),
            Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(e) => Err(failed(e)),
        }
    }

    fn set(&self, id: &str, secret: &[u8]) -> Result<(), ConfigError> {
        let mut options = item(id);
        // The protection class is an attribute of the item, set when
        // it is added. An existing item is updated in place under the
        // bare query: the crate's update reuses the add's query as the
        // search, and an access control object is not a search key.
        if self.get(id)?.is_none() {
            options.set_access_control(
                SecAccessControl::create_with_protection(
                    Some(ProtectionMode::AccessibleAfterFirstUnlockThisDeviceOnly),
                    0,
                )
                .map_err(failed)?,
            );
        }
        set_generic_password_options(secret, options).map_err(failed)
    }

    fn delete(&self, id: &str) -> Result<(), ConfigError> {
        match delete_generic_password_options(item(id)) {
            Err(e) if e.code() != ERR_SEC_ITEM_NOT_FOUND => Err(failed(e)),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test binary carries no application identifier, so the data
    /// protection keychain refuses its writes, and `available` must
    /// say so rather than let the process switch stores: its reads
    /// would come back "no such item" and look like an orphaned
    /// profile. Run under a signed, provisioned host this would not
    /// hold; cargo's is not one.
    #[test]
    fn unentitled_process_is_refused() {
        assert!(!available());
        assert!(matches!(
            Keychain.set("availability-probe", b"x"),
            Err(ConfigError::Keychain(_))
        ));
    }
}
