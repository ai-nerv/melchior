//! Every command a magi starts belongs to that magi and ends with it.

/// Ask the kernel for `SIGTERM` when the magi ends. `PR_SET_PDEATHSIG` watches the spawning
/// *thread* and only from the moment it is set, hence the `getppid` on either side.
pub fn to_magi() -> std::io::Result<()> {
    let magi = rustix::process::getppid();
    rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::Term))?;
    if rustix::process::getppid() != magi {
        std::process::exit(0);
    }
    Ok(())
}
