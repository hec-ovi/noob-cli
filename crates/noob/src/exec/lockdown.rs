//! The folder lock for model-typed commands: Landlock, the kernel's
//! unprivileged access control. A locked child, and everything it starts,
//! keeps reading and executing the whole filesystem but can write only
//! beneath the folders the lock names: the workspace, the shared temp
//! trees, and /dev/null. The kernel enforces it, an exec cannot shed it,
//! and a process the child leaves behind stays locked too.
//!
//! Built once in the parent (one ruleset fd, reused for every spawn),
//! applied in the child between fork and exec. A kernel without Landlock
//! (before 5.13, or an lsm= list that omits it) says so at build time and
//! the caller runs unlocked and says so: that is the best-effort
//! half of the contract.

#[cfg(target_os = "linux")]
pub(crate) use linux::*;

#[cfg(target_os = "linux")]
mod linux {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::path::Path;

    // include/uapi/linux/landlock.h, the write family only. Read and execute
    // rights exist but are deliberately never handled: the wall is about
    // mutation, and an unhandled right is an unrestricted one.
    const WRITE_FILE: u64 = 1 << 1;
    const REMOVE_DIR: u64 = 1 << 4;
    const REMOVE_FILE: u64 = 1 << 5;
    const MAKE_CHAR: u64 = 1 << 6;
    const MAKE_DIR: u64 = 1 << 7;
    const MAKE_REG: u64 = 1 << 8;
    const MAKE_SOCK: u64 = 1 << 9;
    const MAKE_FIFO: u64 = 1 << 10;
    const MAKE_BLOCK: u64 = 1 << 11;
    const MAKE_SYM: u64 = 1 << 12;
    /// ABI 2 (5.19): rename or link across directories.
    const REFER: u64 = 1 << 13;
    /// ABI 3 (6.2): truncate. Unhandled on older kernels, where the right
    /// does not exist and handling it would fail the ruleset.
    const TRUNCATE: u64 = 1 << 14;

    const CREATE_RULESET_VERSION: u32 = 1;
    const RULE_PATH_BENEATH: u32 = 1;

    #[repr(C)]
    struct RulesetAttr {
        handled_access_fs: u64,
    }

    // Packed in the uapi header: a u64 then an i32, twelve bytes, no tail
    // padding. A plain repr(C) would send sixteen and the kernel would
    // reject the size.
    #[repr(C, packed)]
    struct PathBeneath {
        allowed_access: u64,
        parent_fd: RawFd,
    }

    /// The kernel's Landlock ABI level, or a negative errno-style result
    /// when there is none.
    fn abi() -> i64 {
        unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<RulesetAttr>(),
                0usize,
                CREATE_RULESET_VERSION,
            )
        }
    }

    /// One ruleset fd, built in the parent, applied to every locked child.
    pub(crate) struct Lockdown {
        ruleset: OwnedFd,
    }

    impl Lockdown {
        /// Write access beneath `workspace`, `/tmp`, `/var/tmp`, `/dev/shm`,
        /// and to `/dev/null`; everything else stays read-only. The
        /// workspace rule is load-bearing and fails the build; the shared
        /// paths are added when they exist.
        pub(crate) fn for_workspace(workspace: &Path) -> Result<Lockdown, String> {
            let abi = abi();
            if abi < 1 {
                return Err(
                    "the kernel has no Landlock (needs 5.13+ with landlock in lsm=)".to_string(),
                );
            }
            let dir_family = WRITE_FILE
                | REMOVE_DIR
                | REMOVE_FILE
                | MAKE_CHAR
                | MAKE_DIR
                | MAKE_REG
                | MAKE_SOCK
                | MAKE_FIFO
                | MAKE_BLOCK
                | MAKE_SYM
                | if abi >= 2 { REFER } else { 0 }
                | if abi >= 3 { TRUNCATE } else { 0 };
            let attr = RulesetAttr {
                handled_access_fs: dir_family,
            };
            let fd = unsafe {
                libc::syscall(
                    libc::SYS_landlock_create_ruleset,
                    &attr,
                    std::mem::size_of::<RulesetAttr>(),
                    0u32,
                )
            };
            if fd < 0 {
                return Err(format!(
                    "landlock ruleset failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let ruleset = unsafe { OwnedFd::from_raw_fd(fd as RawFd) };
            add_rule(&ruleset, workspace, dir_family, true).map_err(|e| {
                format!("cannot lock commands to {}: {e}", workspace.display())
            })?;
            for shared in ["/tmp", "/var/tmp", "/dev/shm"] {
                let _ = add_rule(&ruleset, Path::new(shared), dir_family, true);
            }
            // A file rule takes file rights only; MAKE_* on a non-directory
            // is EINVAL.
            let _ = add_rule(
                &ruleset,
                Path::new("/dev/null"),
                WRITE_FILE | (dir_family & TRUNCATE),
                false,
            );
            Ok(Lockdown { ruleset })
        }

        /// For the pre-exec closure: a Copy handle the child applies.
        pub(crate) fn raw_fd(&self) -> RawFd {
            self.ruleset.as_raw_fd()
        }
    }

    fn add_rule(
        ruleset: &OwnedFd,
        path: &Path,
        allowed: u64,
        dir: bool,
    ) -> std::io::Result<()> {
        use std::os::unix::ffi::OsStrExt;
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|_| std::io::Error::other("path contains a NUL byte"))?;
        let flags = libc::O_PATH | libc::O_CLOEXEC | if dir { libc::O_DIRECTORY } else { 0 };
        let fd = unsafe { libc::open(c_path.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let parent = unsafe { OwnedFd::from_raw_fd(fd) };
        let rule = PathBeneath {
            allowed_access: allowed,
            parent_fd: parent.as_raw_fd(),
        };
        let rc = unsafe {
            libc::syscall(
                libc::SYS_landlock_add_rule,
                ruleset.as_raw_fd(),
                RULE_PATH_BENEATH,
                &rule,
                0u32,
            )
        };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Between fork and exec, on the child. Raw syscalls only: this runs in
    /// the pre-exec window where allocation is not allowed. no_new_privs is
    /// what Landlock requires of an unprivileged caller, and it also means a
    /// locked command cannot regain ground through setuid binaries.
    pub(crate) fn apply(ruleset_fd: RawFd) -> std::io::Result<()> {
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset_fd, 0u32) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// For `noob doctor`: what the lock would be on this kernel.
    pub(crate) fn support() -> Result<String, String> {
        match abi() {
            n if n >= 1 => Ok(format!("landlock abi {n}")),
            _ => Err("the kernel has no Landlock (needs 5.13+ with landlock in lsm=)".to_string()),
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) use stub::*;

#[cfg(not(target_os = "linux"))]
mod stub {
    use std::path::Path;

    /// No mechanism on this OS yet (macOS gets Seatbelt later); the type
    /// exists so callers compile, and no value of it can.
    pub(crate) struct Lockdown {
        never: std::convert::Infallible,
    }

    impl Lockdown {
        pub(crate) fn for_workspace(_workspace: &Path) -> Result<Lockdown, String> {
            Err("no folder lock on this OS yet".to_string())
        }

        #[cfg(unix)]
        pub(crate) fn raw_fd(&self) -> std::os::fd::RawFd {
            match self.never {}
        }
    }

    #[cfg(unix)]
    pub(crate) fn apply(_ruleset_fd: std::os::fd::RawFd) -> std::io::Result<()> {
        Ok(())
    }

    pub(crate) fn support() -> Result<String, String> {
        Err("no folder lock on this OS yet".to_string())
    }
}
