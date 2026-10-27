use crate::config::Profile;

pub async fn run() -> anyhow::Result<()> {
    match Profile::active()? {
        Some(p) => {
            // Held until the purge is done, so no host (the app's
            // included) boots the doc out from under it.
            let _claim = crate::client::claim(&p.dir).await?;
            // The login agent would fail at every login against a
            // purged profile.
            crate::service::uninstall_quietly();
            p.purge()?;
            println!("Logged out (local state wiped).");
        }
        None => println!("Not logged in."),
    }
    Ok(())
}
