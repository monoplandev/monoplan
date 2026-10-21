use crate::config::Profile;

pub async fn run() -> anyhow::Result<()> {
    match Profile::active()? {
        Some(p) => {
            crate::client::stop_if_running(&p.dir).await?;
            p.purge()?;
            println!("Logged out (local state wiped).");
        }
        None => println!("Not logged in."),
    }
    Ok(())
}
