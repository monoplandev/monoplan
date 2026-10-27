use crate::config::Profile;

pub async fn run() -> anyhow::Result<()> {
    match Profile::active()? {
        Some(p) => {
            // Refuses while Monoplan.app hosts the profile; nothing below
            // runs in that case.
            crate::client::stop_if_running(&p.dir).await?;
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
