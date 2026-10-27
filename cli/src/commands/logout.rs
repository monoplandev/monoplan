use crate::config::Profile;

pub async fn run() -> anyhow::Result<()> {
    match Profile::active()? {
        Some(p) if p.has_account()? => {
            logout(&p).await?;
            println!("Logged out (local state wiped).");
        }
        // A local-only workspace has nothing to log out of, and
        // nothing a sign-out should destroy.
        Some(_) => println!("Not logged in (the local workspace is kept)."),
        None => println!("Not logged in."),
    }
    Ok(())
}

/// Stop the profile's host and wipe the profile. Shared with the app's
/// Sign out (`monoplan-ffi::logout`).
pub async fn logout(p: &Profile) -> anyhow::Result<()> {
    // Held until the purge is done, so no host (the app's included)
    // boots the doc out from under it.
    let _claim = crate::client::claim(&p.dir).await?;
    // The login agent would fail at every login against a purged
    // profile.
    crate::service::uninstall_quietly();
    p.purge()?;
    Ok(())
}
