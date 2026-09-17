//! `tedge-dot pki` run as root on a PKI directory another user owns (the packaged
//! `/var/lib/tedge-dot/opcua/pki` belongs to `tedge`).
//!
//! Root must not touch files in a directory an unprivileged user controls: that user could
//! swap any path component for a symlink between a check and its use, and have root write,
//! chmod or chown files anywhere. So the PKI work runs with the owner's effective user and
//! group IDs (and only the owner's group): everything it creates belongs to the owner, and a
//! symlink can only lead to files the owner could change anyway. Only what the invoking
//! administrator named, the file an action reads and the file `export --output` writes, is
//! accessed as root, through [`as_invoker`]. The C build (`pki_cli.c`) does the same.

use std::path::Path;

/// Restores the invoking identity when dropped.
pub struct AsOwner(());

/// Switch to the owner of `root` when running as root and `root` belongs to another user;
/// otherwise nothing changes.
pub fn enter(root: &Path) -> Result<AsOwner, String> {
    #[cfg(unix)]
    imp::enter(root)?;
    #[cfg(not(unix))]
    let _ = root;
    Ok(AsOwner(()))
}

impl Drop for AsOwner {
    fn drop(&mut self) {
        #[cfg(unix)]
        imp::leave();
    }
}

/// Run `f` with the invoking identity (root), for files the administrator named.
pub fn as_invoker<T>(f: impl FnOnce() -> T) -> T {
    #[cfg(unix)]
    return imp::as_invoker(f);
    #[cfg(not(unix))]
    f()
}

#[cfg(unix)]
mod imp {
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;
    use std::sync::Mutex;

    struct Owner {
        uid: libc::uid_t,
        gid: libc::gid_t,
        saved_egid: libc::gid_t,
        saved_groups: Vec<libc::gid_t>,
    }

    static OWNER: Mutex<Option<Owner>> = Mutex::new(None);

    fn last_error(what: &str) -> String {
        format!("{what}: {}", std::io::Error::last_os_error())
    }

    fn to_owner(o: &Owner) -> Result<(), String> {
        // SAFETY: plain system calls on values we own; the order matters (groups and group
        // first, while still root).
        unsafe {
            if libc::setgroups(1, &o.gid) != 0 {
                return Err(last_error("cannot drop supplementary groups"));
            }
            if libc::setegid(o.gid) != 0 {
                return Err(last_error("cannot switch group"));
            }
            if libc::seteuid(o.uid) != 0 {
                return Err(last_error("cannot switch user"));
            }
        }
        Ok(())
    }

    fn to_root(o: &Owner) -> Result<(), String> {
        // SAFETY: as above; the saved user ID is still 0, so seteuid(0) is permitted.
        unsafe {
            if libc::seteuid(0) != 0 {
                return Err(last_error("cannot switch back to root"));
            }
            if libc::setegid(o.saved_egid) != 0 {
                return Err(last_error("cannot restore the group"));
            }
            if libc::setgroups(o.saved_groups.len() as _, o.saved_groups.as_ptr()) != 0 {
                return Err(last_error("cannot restore supplementary groups"));
            }
        }
        Ok(())
    }

    /// A half-switched identity is not safe to continue with.
    fn fatal(msg: String) -> ! {
        eprintln!("error: {msg}");
        std::process::exit(1);
    }

    pub fn enter(root: &Path) -> Result<(), String> {
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } != 0 {
            return Ok(());
        }
        let Ok(meta) = std::fs::metadata(root) else {
            // Not there yet: root creates it, owned by root.
            return Ok(());
        };
        if meta.uid() == 0 {
            return Ok(());
        }
        // SAFETY: getgroups(0, NULL) returns the count; the buffer is sized to it.
        let saved_groups = unsafe {
            let n = libc::getgroups(0, std::ptr::null_mut());
            if n < 0 {
                return Err(last_error("cannot read supplementary groups"));
            }
            let mut groups = vec![0 as libc::gid_t; n as usize];
            let n = libc::getgroups(n, groups.as_mut_ptr());
            if n < 0 {
                return Err(last_error("cannot read supplementary groups"));
            }
            groups.truncate(n as usize);
            groups
        };
        let owner = Owner {
            uid: meta.uid(),
            gid: meta.gid(),
            // SAFETY: getegid has no preconditions.
            saved_egid: unsafe { libc::getegid() },
            saved_groups,
        };
        let mut slot = OWNER.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = to_owner(&owner) {
            // Undo a partial switch before reporting.
            if let Err(undo) = to_root(&owner) {
                fatal(undo);
            }
            return Err(format!("{e} (to act as the owner of {})", root.display()));
        }
        *slot = Some(owner);
        Ok(())
    }

    pub fn leave() {
        let mut slot = OWNER.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(owner) = slot.take() {
            if let Err(e) = to_root(&owner) {
                fatal(e);
            }
        }
    }

    pub fn as_invoker<T>(f: impl FnOnce() -> T) -> T {
        let slot = OWNER.lock().unwrap_or_else(|e| e.into_inner());
        let Some(owner) = slot.as_ref() else {
            return f();
        };
        if let Err(e) = to_root(owner) {
            fatal(e);
        }
        let result = f();
        if let Err(e) = to_owner(owner) {
            fatal(e);
        }
        result
    }
}
