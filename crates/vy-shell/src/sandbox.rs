//! The OS sandbox of the tab process (SPEC-9.1.1, SPEC-9.1.3, SPEC-11.2.2).
//!
//! A tab process needs, after startup, its two pipes, memory, signals and
//! a clock; everything else it might reach is the operating system's.
//! [`lockdown`] takes the rest away from the child itself, before the
//! first message from the shell is read, so no page code ever runs with
//! more. What the tab does after that, measured with `strace` on Linux:
//! `read(0)`, `write(1|2)`, `mmap`/`mprotect`/`munmap`/`brk` (wasmtime's
//! code and linear memories), `memfd_create` + `write` + `fcntl` (wasmtime's
//! copy-on-write memory images), `sigaltstack`/`rt_sigaction` (wasmtime's
//! trap handlers), `getrandom`, `getpid`/`gettid`, `exit_group`. Fonts are
//! compiled into the binary, storage and fetches go to the main process
//! over IPC, so nothing in the tab needs a file or a socket.
//!
//! - **Linux:** every file descriptor above 2 is closed (nothing the shell
//!   leaked without `O_CLOEXEC`, such as a display connection, survives),
//!   `PR_SET_NO_NEW_PRIVS`, Landlock with every filesystem right (and,
//!   from ABI 4, TCP, from ABI 6 abstract sockets and signals) handled and
//!   none granted when the kernel has it (best effort, silent otherwise),
//!   then a seccomp-bpf allowlist of the calls above; anything else fails
//!   with `EPERM`, `clone` only makes threads, `tgkill` only reaches this
//!   process.
//! - **Windows:** the shell puts the child in a job object (kill on close,
//!   one active process, a memory limit, every UI restriction, no crash
//!   dialog); the child lowers its own token to low integrity (no write
//!   access to the user's files and registry) and turns on the mitigation
//!   policies that cannot break wasmtime: no extension-point DLLs, no
//!   images from remote shares or low-integrity files, no non-system
//!   fonts. With `VY_SANDBOX=strict` the shell instead starts the child in
//!   an AppContainer without capabilities (no network, no user files),
//!   inheriting only its pipe handles, with child processes forbidden by
//!   policy, and the child adds strict handle checks; see
//!   `docs/notes/m4-security.md` for what to observe.
//! - **macOS:** `sandbox_init` with a deny-default profile; if the call
//!   fails the tab runs unsandboxed and says so.
//!
//! `VY_SANDBOX=off` turns the child-side lockdown off for debugging; the
//! job object on Windows stays. `vy-shell --tab --sandbox-probe` locks
//! down and then tries to read a file, write one, open a socket and start
//! a process, and reports what was refused ([`probe`]).

use std::fmt;

/// How strictly to sandbox, from `VY_SANDBOX`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `VY_SANDBOX=off`: no child-side lockdown.
    Off,
    /// The default: what is known not to break wasmtime or the pipes.
    Default,
    /// `VY_SANDBOX=strict`: everything, including what cannot be tried
    /// on every platform from here (Windows AppContainer).
    Strict,
}

impl Mode {
    /// The mode `VY_SANDBOX` asks for.
    pub fn from_env() -> Mode {
        match std::env::var("VY_SANDBOX").as_deref() {
            Ok("off") => Mode::Off,
            Ok("strict") => Mode::Strict,
            _ => Mode::Default,
        }
    }
}

/// What [`lockdown`] put in place.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Layers in force, in the order they were applied.
    pub layers: Vec<String>,
    /// Layers that could not be applied, with the reason.
    pub warnings: Vec<String>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.layers.is_empty() {
            f.write_str("none")?;
        } else {
            f.write_str(&self.layers.join(", "))?;
        }
        for w in &self.warnings {
            write!(f, "; not applied: {w}")?;
        }
        Ok(())
    }
}

/// Lock the current process down as a tab process (SPEC-9.1.3). Call once,
/// after startup and before anything from the shell is read; there is no
/// way back.
pub fn lockdown(mode: Mode) -> Report {
    let mut report = Report::default();
    if mode == Mode::Off {
        report.warnings.push("VY_SANDBOX=off".into());
        return report;
    }
    platform::lockdown(mode, &mut report);
    report
}

/// One attempt of the probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attempt {
    /// What was tried.
    pub what: &'static str,
    /// `Err(reason)` when it was refused.
    pub outcome: Result<(), String>,
}

/// After [`lockdown`]: try to read `exe`, create a file in the temp
/// directory, open a socket and start `exe` again. Every attempt should be
/// refused; on Windows outside strict mode reading and the socket are not
/// (see the module text).
pub fn probe(exe: &std::path::Path) -> Vec<Attempt> {
    use std::io::Read;
    let read = std::fs::File::open(exe)
        .and_then(|mut f| f.read(&mut [0u8; 16]))
        .map(|_| ())
        .map_err(|e| e.to_string());
    let target = std::env::temp_dir().join(format!("vy-sandbox-probe-{}", std::process::id()));
    let write = std::fs::write(&target, b"x").map_err(|e| e.to_string());
    if write.is_ok() {
        let _ = std::fs::remove_file(&target);
    }
    let socket = std::net::UdpSocket::bind("127.0.0.1:0")
        .map(|_| ())
        .map_err(|e| e.to_string());
    let process = std::process::Command::new(exe)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|mut c| {
            let _ = c.kill();
            let _ = c.wait();
        })
        .map_err(|e| e.to_string());
    vec![
        Attempt {
            what: "file-read",
            outcome: read,
        },
        Attempt {
            what: "file-write",
            outcome: write,
        },
        Attempt {
            what: "socket",
            outcome: socket,
        },
        Attempt {
            what: "process",
            outcome: process,
        },
    ]
}

/// The probe's report, one line per attempt (`<what>: refused (<why>)` or
/// `<what>: allowed`) after a `sandbox:` line.
pub fn probe_text(report: &Report, attempts: &[Attempt]) -> String {
    let mut out = format!("sandbox: {report}\n");
    for a in attempts {
        match &a.outcome {
            Err(why) => out.push_str(&format!("{}: refused ({why})\n", a.what)),
            Ok(()) => out.push_str(&format!("{}: allowed\n", a.what)),
        }
    }
    out
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{Mode, Report};
    use std::io;

    // Classic BPF (linux/filter.h, linux/bpf_common.h).
    const BPF_LD_W_ABS: u16 = 0x20;
    const BPF_JEQ_K: u16 = 0x15;
    const BPF_JGE_K: u16 = 0x35;
    const BPF_JSET_K: u16 = 0x45;
    const BPF_RET_K: u16 = 0x06;
    // linux/seccomp.h
    const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
    const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
    const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
    const SECCOMP_SET_MODE_FILTER: libc::c_ulong = 1;
    const SECCOMP_FILTER_FLAG_TSYNC: libc::c_ulong = 1;
    // struct seccomp_data: nr at 0, arch at 4, args at 16 + 8 i (low half
    // first on little-endian).
    const OFF_NR: u32 = 0;
    const OFF_ARCH: u32 = 4;
    const OFF_ARG0: u32 = 16;
    #[cfg(target_arch = "x86_64")]
    const AUDIT_ARCH: u32 = 0xC000_003E;
    #[cfg(target_arch = "aarch64")]
    const AUDIT_ARCH: u32 = 0xC000_00B7;
    const CLONE_THREAD: u32 = 0x0001_0000;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct SockFilter {
        code: u16,
        jt: u8,
        jf: u8,
        k: u32,
    }

    #[repr(C)]
    struct SockFprog {
        len: u16,
        filter: *const SockFilter,
    }

    fn stmt(code: u16, k: u32) -> SockFilter {
        SockFilter {
            code,
            jt: 0,
            jf: 0,
            k,
        }
    }

    /// The calls a locked-down tab may make (see the module text); each
    /// was seen in a trace or is what std, glibc or wasmtime fall back to.
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    fn allowed() -> Vec<libc::c_long> {
        let mut v = vec![
            libc::SYS_read,
            libc::SYS_write,
            libc::SYS_readv,
            libc::SYS_writev,
            libc::SYS_close,
            libc::SYS_lseek,
            libc::SYS_mmap,
            libc::SYS_mprotect,
            libc::SYS_munmap,
            libc::SYS_mremap,
            libc::SYS_madvise,
            libc::SYS_brk,
            libc::SYS_memfd_create,
            libc::SYS_ftruncate,
            libc::SYS_fcntl,
            libc::SYS_rt_sigaction,
            libc::SYS_rt_sigprocmask,
            libc::SYS_rt_sigreturn,
            libc::SYS_sigaltstack,
            libc::SYS_getpid,
            libc::SYS_gettid,
            libc::SYS_getrandom,
            libc::SYS_clock_gettime,
            libc::SYS_clock_getres,
            libc::SYS_clock_nanosleep,
            libc::SYS_nanosleep,
            libc::SYS_sched_yield,
            libc::SYS_sched_getaffinity,
            libc::SYS_futex,
            libc::SYS_set_robust_list,
            libc::SYS_rseq,
            libc::SYS_membarrier,
            libc::SYS_prlimit64,
            libc::SYS_restart_syscall,
            libc::SYS_exit,
            libc::SYS_exit_group,
        ];
        #[cfg(target_arch = "x86_64")]
        v.push(libc::SYS_arch_prctl);
        v
    }

    /// The filter: kill on a foreign architecture (or the x32 ABI), allow
    /// the list, `clone` only with `CLONE_THREAD`, `tgkill` only to `pid`,
    /// `clone3` is `ENOSYS` (so libc falls back to the `clone` checked
    /// here), everything else `EPERM`.
    fn filter(pid: u32) -> Vec<SockFilter> {
        let list = allowed();
        let mut f = vec![
            stmt(BPF_LD_W_ABS, OFF_ARCH),
            SockFilter {
                code: BPF_JEQ_K,
                jt: 1,
                jf: 0,
                k: AUDIT_ARCH,
            },
            stmt(BPF_RET_K, SECCOMP_RET_KILL_PROCESS),
            stmt(BPF_LD_W_ABS, OFF_NR),
        ];
        if cfg!(target_arch = "x86_64") {
            // x32 calls carry bit 30.
            f.push(SockFilter {
                code: BPF_JGE_K,
                jt: 0,
                jf: 1,
                k: 0x4000_0000,
            });
            f.push(stmt(BPF_RET_K, SECCOMP_RET_KILL_PROCESS));
        }
        // Layout after the checks: [deny, allow, clone check..., tgkill
        // check..., enosys]. Each list entry jumps to `allow`.
        let checks = list.len() + 3; // the list, clone, tgkill, clone3
        let base = f.len();
        let deny = base + checks;
        let allow = deny + 1;
        let clone_at = allow + 1; // ld arg0; jset; ret allow; ret deny
        let tgkill_at = clone_at + 4; // ld arg0; jeq pid; ret allow; ret deny
        let enosys = tgkill_at + 4;
        let jump = |from: usize, to: usize| -> u8 {
            u8::try_from(to - from - 1).expect("filter jump fits in 8 bits")
        };
        for (i, nr) in list.iter().enumerate() {
            let at = base + i;
            f.push(SockFilter {
                code: BPF_JEQ_K,
                jt: jump(at, allow),
                jf: 0,
                k: *nr as u32,
            });
        }
        for (nr, to) in [
            (libc::SYS_clone, clone_at),
            (libc::SYS_tgkill, tgkill_at),
            (libc::SYS_clone3, enosys),
        ] {
            let at = f.len();
            f.push(SockFilter {
                code: BPF_JEQ_K,
                jt: jump(at, to),
                jf: 0,
                k: nr as u32,
            });
        }
        debug_assert_eq!(f.len(), deny);
        f.push(stmt(BPF_RET_K, SECCOMP_RET_ERRNO | libc::EPERM as u32));
        f.push(stmt(BPF_RET_K, SECCOMP_RET_ALLOW));
        // clone: the flags are the first argument on both architectures.
        f.push(stmt(BPF_LD_W_ABS, OFF_ARG0));
        f.push(SockFilter {
            code: BPF_JSET_K,
            jt: 0,
            jf: 1,
            k: CLONE_THREAD,
        });
        f.push(stmt(BPF_RET_K, SECCOMP_RET_ALLOW));
        f.push(stmt(BPF_RET_K, SECCOMP_RET_ERRNO | libc::EPERM as u32));
        // tgkill(tgid, ...): only this process.
        f.push(stmt(BPF_LD_W_ABS, OFF_ARG0));
        f.push(SockFilter {
            code: BPF_JEQ_K,
            jt: 0,
            jf: 1,
            k: pid,
        });
        f.push(stmt(BPF_RET_K, SECCOMP_RET_ALLOW));
        f.push(stmt(BPF_RET_K, SECCOMP_RET_ERRNO | libc::EPERM as u32));
        debug_assert_eq!(f.len(), enosys);
        f.push(stmt(BPF_RET_K, SECCOMP_RET_ERRNO | libc::ENOSYS as u32));
        f
    }

    fn seccomp() -> io::Result<()> {
        let filter = filter(std::process::id());
        let prog = SockFprog {
            len: u16::try_from(filter.len()).expect("filter length"),
            filter: filter.as_ptr(),
        };
        // SAFETY: `prog` points at `filter`, which outlives the call; the
        // kernel copies the program.
        let r = unsafe {
            libc::syscall(
                libc::SYS_seccomp,
                SECCOMP_SET_MODE_FILTER,
                SECCOMP_FILTER_FLAG_TSYNC,
                &prog as *const SockFprog,
            )
        };
        if r == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    // linux/landlock.h
    const LANDLOCK_CREATE_RULESET_VERSION: u32 = 1;

    #[repr(C)]
    struct RulesetAttr {
        handled_access_fs: u64,
        handled_access_net: u64,
        scoped: u64,
    }

    /// Landlock with every right the kernel knows handled and none
    /// granted: no path can be opened, from ABI 4 no TCP bind or connect,
    /// from ABI 6 no abstract unix socket or signal outside the domain.
    fn landlock() -> io::Result<i64> {
        // SAFETY: the version query takes no attribute.
        let abi = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                std::ptr::null::<RulesetAttr>(),
                0usize,
                LANDLOCK_CREATE_RULESET_VERSION,
            )
        };
        if abi < 1 {
            return Err(io::Error::last_os_error());
        }
        let fs = match abi {
            1 => (1u64 << 13) - 1,
            2 => (1 << 14) - 1,
            3 | 4 => (1 << 15) - 1,
            _ => (1 << 16) - 1,
        };
        let attr = RulesetAttr {
            handled_access_fs: fs,
            handled_access_net: if abi >= 4 { 0b11 } else { 0 },
            scoped: if abi >= 6 { 0b11 } else { 0 },
        };
        // SAFETY: `attr` is a valid ruleset attribute of the size passed;
        // a kernel that knows fewer fields accepts the zeroed rest.
        let fd = unsafe {
            libc::syscall(
                libc::SYS_landlock_create_ruleset,
                &attr as *const RulesetAttr,
                std::mem::size_of::<RulesetAttr>(),
                0u32,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `fd` is the ruleset just created; it is closed after.
        let r = unsafe { libc::syscall(libc::SYS_landlock_restrict_self, fd as libc::c_int, 0u32) };
        let err = io::Error::last_os_error();
        // SAFETY: closing the descriptor this function owns.
        unsafe { libc::close(fd as libc::c_int) };
        if r != 0 {
            return Err(err);
        }
        Ok(abi)
    }

    /// Close every descriptor above stderr: what the shell's process held
    /// without close-on-exec must not reach page code.
    fn close_inherited() {
        // SAFETY: close_range only closes descriptors.
        let r = unsafe { libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0u32) };
        if r != 0 {
            for fd in 3..4096 {
                // SAFETY: closing a descriptor number that may be unused.
                unsafe { libc::close(fd) };
            }
        }
    }

    pub fn lockdown(_mode: Mode, report: &mut Report) {
        close_inherited();
        report.layers.push("inherited descriptors closed".into());
        // SAFETY: plain prctl with integer arguments.
        if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
            report.warnings.push(format!(
                "no_new_privs: {}; no seccomp or Landlock without it",
                io::Error::last_os_error()
            ));
            return;
        }
        report.layers.push("no_new_privs".into());
        match landlock() {
            Ok(abi) => report.layers.push(format!("landlock (abi {abi})")),
            Err(e) => report.warnings.push(format!("landlock: {e}")),
        }
        match seccomp() {
            Ok(()) => report.layers.push("seccomp allowlist".into()),
            Err(e) => report.warnings.push(format!("seccomp: {e}")),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The filter's jumps land where they should: every listed call
        /// at the allow return, the special cases at their checks.
        #[test]
        fn filter_layout() {
            let f = filter(42);
            let allow = f
                .iter()
                .position(|i| i.code == BPF_RET_K && i.k == SECCOMP_RET_ALLOW)
                .unwrap();
            for (at, i) in f.iter().enumerate() {
                if i.code == BPF_JEQ_K && allowed().contains(&(i.k as libc::c_long)) && at < allow {
                    assert_eq!(at + 1 + i.jt as usize, allow, "{}", i.k);
                }
            }
            assert_eq!(f.last().unwrap().k, SECCOMP_RET_ERRNO | libc::ENOSYS as u32);
            assert!(f.len() < 100);
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::{Mode, Report};
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
    use windows_sys::Win32::Security::{
        CreateWellKnownSid, SetTokenInformation, TokenIntegrityLevel, WinLowLabelSid,
        SID_AND_ATTRIBUTES, TOKEN_ADJUST_DEFAULT, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::SystemServices::{
        PROCESS_MITIGATION_EXTENSION_POINT_DISABLE_POLICY, PROCESS_MITIGATION_FONT_DISABLE_POLICY,
        PROCESS_MITIGATION_IMAGE_LOAD_POLICY, PROCESS_MITIGATION_STRICT_HANDLE_CHECK_POLICY,
        SE_GROUP_INTEGRITY,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcessToken, ProcessExtensionPointDisablePolicy,
        ProcessFontDisablePolicy, ProcessImageLoadPolicy, ProcessStrictHandleCheckPolicy,
        SetProcessMitigationPolicy, PROCESS_MITIGATION_POLICY,
    };

    fn last_error() -> String {
        // SAFETY: reads the thread's last-error value.
        format!("error {}", unsafe { GetLastError() })
    }

    /// Lower this process's token to low integrity: no write access to
    /// objects of the user's medium level (files, registry) from now on.
    fn low_integrity() -> Result<(), String> {
        let mut sid = [0u8; 68]; // SECURITY_MAX_SID_SIZE
        let mut len = sid.len() as u32;
        // SAFETY: `sid` is a buffer of `len` bytes for the well-known SID.
        if unsafe {
            CreateWellKnownSid(
                WinLowLabelSid,
                std::ptr::null_mut(),
                sid.as_mut_ptr().cast(),
                &mut len,
            )
        } == 0
        {
            return Err(format!("CreateWellKnownSid: {}", last_error()));
        }
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: opening this process's own token with an out pointer.
        if unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_DEFAULT | TOKEN_QUERY,
                &mut token,
            )
        } == 0
        {
            return Err(format!("OpenProcessToken: {}", last_error()));
        }
        let label = TOKEN_MANDATORY_LABEL {
            Label: SID_AND_ATTRIBUTES {
                Sid: sid.as_mut_ptr().cast(),
                Attributes: SE_GROUP_INTEGRITY as u32,
            },
        };
        let size = std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 + len;
        // SAFETY: `label` points at `sid`, both alive for the call; the
        // token handle is closed after.
        let ok = unsafe {
            SetTokenInformation(
                token,
                TokenIntegrityLevel,
                (&label as *const TOKEN_MANDATORY_LABEL).cast(),
                size,
            )
        };
        let err = last_error();
        // SAFETY: closing the token handle opened above.
        unsafe { CloseHandle(token) };
        if ok == 0 {
            return Err(format!("SetTokenInformation: {err}"));
        }
        Ok(())
    }

    fn mitigate(policy: PROCESS_MITIGATION_POLICY, flags: u32, name: &str, report: &mut Report) {
        // Every policy structure used here is one 32-bit flag word.
        let value = flags;
        // SAFETY: a 4-byte policy value of the size passed.
        let ok = unsafe { SetProcessMitigationPolicy(policy, (&value as *const u32).cast(), 4) };
        if ok == 0 {
            report.warnings.push(format!("{name}: {}", last_error()));
        } else {
            report.layers.push(name.to_string());
        }
    }

    pub fn lockdown(mode: Mode, report: &mut Report) {
        const _: () = assert!(
            std::mem::size_of::<PROCESS_MITIGATION_EXTENSION_POINT_DISABLE_POLICY>() == 4
                && std::mem::size_of::<PROCESS_MITIGATION_IMAGE_LOAD_POLICY>() == 4
                && std::mem::size_of::<PROCESS_MITIGATION_FONT_DISABLE_POLICY>() == 4
                && std::mem::size_of::<PROCESS_MITIGATION_STRICT_HANDLE_CHECK_POLICY>() == 4
        );
        // DisableExtensionPoints (bit 0): no AppInit DLLs, winsock LSPs.
        mitigate(
            ProcessExtensionPointDisablePolicy,
            1,
            "no extension points",
            report,
        );
        // NoRemoteImages | NoLowMandatoryLabelImages (bits 0, 1).
        mitigate(
            ProcessImageLoadPolicy,
            0b11,
            "no remote or low-label images",
            report,
        );
        // DisableNonSystemFonts (bit 0).
        mitigate(ProcessFontDisablePolicy, 1, "no non-system fonts", report);
        if mode == Mode::Strict {
            // RaiseExceptionOnInvalidHandleReference |
            // HandleExceptionsPermanentlyEnabled (bits 0, 1).
            mitigate(
                ProcessStrictHandleCheckPolicy,
                0b11,
                "strict handle checks",
                report,
            );
        }
        match low_integrity() {
            Ok(()) => report.layers.push("low integrity".into()),
            Err(e) => report.warnings.push(format!("low integrity: {e}")),
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{Mode, Report};
    use std::ffi::{c_char, c_int, CStr};

    extern "C" {
        fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;
        fn sandbox_free_error(errorbuf: *mut c_char);
    }

    /// Deny everything but what a running tab does to itself.
    const PROFILE: &CStr = c"(version 1)
(deny default)
(allow signal (target self))
(allow process-info* (target self))
(allow sysctl-read)";

    pub fn lockdown(_mode: Mode, report: &mut Report) {
        for fd in 3..1024 {
            // SAFETY: closing a descriptor number that may be unused.
            unsafe { libc::close(fd) };
        }
        report.layers.push("inherited descriptors closed".into());
        let mut err: *mut c_char = std::ptr::null_mut();
        // SAFETY: a NUL-terminated profile and an out pointer for the
        // error, freed with the matching call.
        let r = unsafe { sandbox_init(PROFILE.as_ptr(), 0, &mut err) };
        if r == 0 {
            report.layers.push("sandbox_init deny-default".into());
        } else {
            let msg = if err.is_null() {
                format!("sandbox_init returned {r}")
            } else {
                // SAFETY: `err` is the NUL-terminated message the call set.
                let m = unsafe { CStr::from_ptr(err) }
                    .to_string_lossy()
                    .into_owned();
                // SAFETY: freeing the message once.
                unsafe { sandbox_free_error(err) };
                m
            };
            report
                .warnings
                .push(format!("sandbox_init: {msg}; running unsandboxed"));
        }
    }
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
mod platform {
    use super::{Mode, Report};

    pub fn lockdown(_mode: Mode, report: &mut Report) {
        report.warnings.push("no sandbox on this platform".into());
    }
}

#[cfg(windows)]
pub use job::{contain, spawn_strict, StrictChild, TabJob};

/// The shell's side on Windows: the job object every tab process runs in,
/// and the strict spawn into an AppContainer.
#[cfg(windows)]
mod job {
    use std::io;
    use std::os::windows::io::{FromRawHandle, RawHandle};
    use std::path::Path;
    use windows_sys::Win32::Foundation::{
        CloseHandle, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
        WAIT_OBJECT_0,
    };
    use windows_sys::Win32::Security::Isolation::{
        CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
    };
    use windows_sys::Win32::Security::{FreeSid, PSID, SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicUIRestrictions,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    };
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::SystemServices::JOB_OBJECT_UILIMIT_ALL;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
        InitializeProcThreadAttributeList, ResumeThread, TerminateProcess,
        UpdateProcThreadAttribute, WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED,
        EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
        PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
        PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    };
    use windows_sys::Win32::System::WindowsProgramming::PROCESS_CREATION_CHILD_PROCESS_RESTRICTED;

    /// Memory a tab process may commit: the largest `memory_max` (4 096
    /// pages, 256 MB, SPEC-8.1.3) plus headroom for wasmtime, the compiled
    /// code and decoded images. [`TabJob::limit_memory`] tightens it once
    /// the page's own `memory_max` is known.
    pub const DEFAULT_MEMORY_LIMIT: usize = (256 << 20) + (1 << 30);
    /// Headroom over the page's `memory_max`.
    pub const MEMORY_HEADROOM: usize = 1 << 30;

    fn err(what: &str) -> io::Error {
        let e = io::Error::last_os_error();
        io::Error::new(e.kind(), format!("{what}: {e}"))
    }

    /// A job object holding one tab process. Dropping it kills the process
    /// (`KILL_ON_JOB_CLOSE`), so a crashed shell leaves no tab behind.
    pub struct TabJob(HANDLE);

    // SAFETY: a job handle may be used from any thread.
    unsafe impl Send for TabJob {}

    impl TabJob {
        /// A new job: kill on close, one active process (no children), a
        /// commit limit, no crash dialog, every UI restriction.
        pub fn new() -> io::Result<TabJob> {
            // SAFETY: an anonymous job with default security.
            let h = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if h.is_null() {
                return Err(err("CreateJobObjectW"));
            }
            let job = TabJob(h);
            job.set_limits(DEFAULT_MEMORY_LIMIT)?;
            let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
                UIRestrictionsClass: JOB_OBJECT_UILIMIT_ALL,
            };
            // SAFETY: a UI restriction structure of the size passed.
            if unsafe {
                SetInformationJobObject(
                    job.0,
                    JobObjectBasicUIRestrictions,
                    (&ui as *const JOBOBJECT_BASIC_UI_RESTRICTIONS).cast(),
                    std::mem::size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32,
                )
            } == 0
            {
                return Err(err("JobObjectBasicUIRestrictions"));
            }
            Ok(job)
        }

        fn set_limits(&self, memory: usize) -> io::Result<()> {
            // SAFETY: an all-zero limit structure is valid.
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
                | JOB_OBJECT_LIMIT_PROCESS_MEMORY
                | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
            info.BasicLimitInformation.ActiveProcessLimit = 1;
            info.ProcessMemoryLimit = memory;
            // SAFETY: a limit structure of the size passed.
            if unsafe {
                SetInformationJobObject(
                    self.0,
                    JobObjectExtendedLimitInformation,
                    (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            } == 0
            {
                return Err(err("JobObjectExtendedLimitInformation"));
            }
            Ok(())
        }

        /// Limit the process to `memory_max_pages` of linear memory plus
        /// [`MEMORY_HEADROOM`] (SPEC-9.3.1).
        pub fn limit_memory(&self, memory_max_pages: u32) -> io::Result<()> {
            self.set_limits((memory_max_pages as usize) * 65_536 + MEMORY_HEADROOM)
        }

        fn assign(&self, process: HANDLE) -> io::Result<()> {
            // SAFETY: both handles are valid; the process is ours.
            if unsafe { AssignProcessToJobObject(self.0, process) } == 0 {
                return Err(err("AssignProcessToJobObject"));
            }
            Ok(())
        }
    }

    impl Drop for TabJob {
        fn drop(&mut self) {
            // SAFETY: closing the job handle this value owns; with
            // KILL_ON_JOB_CLOSE the process ends with it.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// Put a just-spawned tab process in a new job. The child has read
    /// nothing from the shell yet, so no page code ran outside the job.
    pub fn contain(child: &std::process::Child) -> io::Result<TabJob> {
        use std::os::windows::io::AsRawHandle;
        let job = TabJob::new()?;
        job.assign(child.as_raw_handle() as HANDLE)?;
        Ok(job)
    }

    /// A tab process started by [`spawn_strict`].
    pub struct StrictChild {
        process: HANDLE,
        pid: u32,
        /// The job; dropping it ends the process.
        pub job: TabJob,
    }

    // SAFETY: process handles may be used from any thread.
    unsafe impl Send for StrictChild {}

    impl StrictChild {
        /// The process id.
        pub fn id(&self) -> u32 {
            self.pid
        }

        /// End the process.
        pub fn kill(&mut self) -> io::Result<()> {
            // SAFETY: terminating our own child.
            if unsafe { TerminateProcess(self.process, 1) } == 0 {
                return Err(err("TerminateProcess"));
            }
            Ok(())
        }

        /// The exit code once the process has ended.
        pub fn try_wait(&mut self) -> io::Result<Option<String>> {
            // SAFETY: a zero-timeout wait on our own process handle.
            if unsafe { WaitForSingleObject(self.process, 0) } != WAIT_OBJECT_0 {
                return Ok(None);
            }
            let mut code = 0u32;
            // SAFETY: reading the exit code of an ended process.
            if unsafe { GetExitCodeProcess(self.process, &mut code) } == 0 {
                return Err(err("GetExitCodeProcess"));
            }
            Ok(Some(format!("exit code: {code}")))
        }

        /// Wait for the end.
        pub fn wait(&mut self) {
            // SAFETY: an infinite wait on our own process handle.
            unsafe { WaitForSingleObject(self.process, u32::MAX) };
        }
    }

    impl Drop for StrictChild {
        fn drop(&mut self) {
            // SAFETY: closing the process handle this value owns.
            unsafe { CloseHandle(self.process) };
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// The AppContainer SID of the tab profile, created on first use.
    fn container_sid() -> io::Result<PSID> {
        let name = wide("vy.tab");
        let display = wide("Vy tab process");
        let mut sid: PSID = std::ptr::null_mut();
        // SAFETY: valid strings, no capabilities, an out pointer.
        let hr = unsafe {
            CreateAppContainerProfile(
                name.as_ptr(),
                display.as_ptr(),
                display.as_ptr(),
                std::ptr::null(),
                0,
                &mut sid,
            )
        };
        if hr >= 0 {
            return Ok(sid);
        }
        // Already there (HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)): derive.
        // SAFETY: a valid name and an out pointer.
        let hr = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
        if hr < 0 {
            return Err(io::Error::other(format!(
                "AppContainer profile: HRESULT {hr:#x}"
            )));
        }
        Ok(sid)
    }

    fn pipe() -> io::Result<(HANDLE, HANDLE)> {
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        let (mut r, mut w): (HANDLE, HANDLE) = (std::ptr::null_mut(), std::ptr::null_mut());
        // SAFETY: out pointers and a valid security attribute.
        if unsafe { CreatePipe(&mut r, &mut w, &sa, 0) } == 0 {
            return Err(err("CreatePipe"));
        }
        Ok((r, w))
    }

    fn no_inherit(h: HANDLE) -> io::Result<()> {
        // SAFETY: a handle this function's caller owns.
        if unsafe { SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0) } == 0 {
            return Err(err("SetHandleInformation"));
        }
        Ok(())
    }

    /// Start `exe <args>` (`--tab`, plain flags) in the `vy.tab` AppContainer without capabilities
    /// (no network, no user files), suspended, with exactly its stdin,
    /// stdout and stderr handles inheritable (`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`),
    /// child processes forbidden, in a new job, then resume it. Returns
    /// the child and the shell's ends of the pipes. The executable must be
    /// readable by `ALL APPLICATION PACKAGES` (true under Program Files).
    pub fn spawn_strict(
        exe: &Path,
        args: &str,
    ) -> io::Result<(StrictChild, std::fs::File, std::fs::File)> {
        let (child_in, shell_in) = pipe()?;
        let (shell_out, child_out) = pipe()?;
        no_inherit(shell_in)?;
        no_inherit(shell_out)?;
        let nul = wide("NUL");
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        // SAFETY: opening the null device with an inheritable handle.
        let child_err = unsafe {
            CreateFileW(
                nul.as_ptr(),
                FILE_GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                &sa,
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        let close_all = |hs: &[HANDLE]| {
            for &h in hs {
                if !h.is_null() && h != INVALID_HANDLE_VALUE {
                    // SAFETY: handles created above and not handed out.
                    unsafe { CloseHandle(h) };
                }
            }
        };
        if child_err == INVALID_HANDLE_VALUE {
            let e = err("CreateFileW(NUL)");
            close_all(&[child_in, shell_in, shell_out, child_out]);
            return Err(e);
        }
        let result = (|| -> io::Result<(StrictChild, HANDLE, HANDLE)> {
            // The job first: a process created suspended must never be
            // left without one.
            let job = TabJob::new()?;
            let sid = container_sid()?;
            let caps = SECURITY_CAPABILITIES {
                AppContainerSid: sid,
                Capabilities: std::ptr::null_mut(),
                CapabilityCount: 0,
                Reserved: 0,
            };
            let mut size = 0usize;
            // SAFETY: the size query of an attribute list of three entries.
            unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 3, 0, &mut size) };
            let mut list_buf = vec![0u8; size];
            let list: LPPROC_THREAD_ATTRIBUTE_LIST = list_buf.as_mut_ptr().cast();
            // SAFETY: `list_buf` has the size the query returned.
            if unsafe { InitializeProcThreadAttributeList(list, 3, 0, &mut size) } == 0 {
                // SAFETY: the SID came from the AppContainer functions.
                unsafe { FreeSid(sid) };
                return Err(err("InitializeProcThreadAttributeList"));
            }
            let handles = [child_in, child_out, child_err];
            let policy: u32 = PROCESS_CREATION_CHILD_PROCESS_RESTRICTED;
            let mut ok = true;
            // SAFETY: each value outlives CreateProcessW below and has the
            // size passed.
            unsafe {
                ok &= UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
                    (&caps as *const SECURITY_CAPABILITIES).cast(),
                    std::mem::size_of::<SECURITY_CAPABILITIES>(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                ) != 0;
                ok &= UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    handles.as_ptr().cast(),
                    std::mem::size_of_val(&handles),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                ) != 0;
                ok &= UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY as usize,
                    (&policy as *const u32).cast(),
                    4,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                ) != 0;
            }
            // SAFETY: an all-zero startup structure is valid.
            let mut si: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
            si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
            si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            si.StartupInfo.hStdInput = child_in;
            si.StartupInfo.hStdOutput = child_out;
            si.StartupInfo.hStdError = child_err;
            si.lpAttributeList = list;
            let app = wide(&exe.to_string_lossy());
            let mut cmdline = wide(&format!("\"{}\" {args}", exe.to_string_lossy()));
            // SAFETY: an all-zero process information structure is valid.
            let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
            let created = ok
                // SAFETY: every pointer is valid for the call; the command
                // line buffer is mutable as CreateProcessW requires.
                && unsafe {
                    CreateProcessW(
                        app.as_ptr(),
                        cmdline.as_mut_ptr(),
                        std::ptr::null(),
                        std::ptr::null(),
                        1,
                        EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | CREATE_NO_WINDOW,
                        std::ptr::null(),
                        std::ptr::null(),
                        &si.StartupInfo,
                        &mut pi,
                    )
                } != 0;
            let e = err("CreateProcessW in AppContainer");
            // SAFETY: the list was initialized above; the SID is ours.
            unsafe {
                DeleteProcThreadAttributeList(list);
                FreeSid(sid);
            }
            if !created {
                return Err(e);
            }
            let child = StrictChild {
                process: pi.hProcess,
                pid: pi.dwProcessId,
                job,
            };
            if let Err(e) = child.job.assign(pi.hProcess) {
                // SAFETY: our own suspended child and its thread handle.
                unsafe {
                    TerminateProcess(pi.hProcess, 1);
                    CloseHandle(pi.hThread);
                }
                return Err(e);
            }
            // SAFETY: resuming and closing the primary thread handle.
            unsafe {
                ResumeThread(pi.hThread);
                CloseHandle(pi.hThread);
            }
            Ok((child, shell_in, shell_out))
        })();
        close_all(&[child_in, child_out, child_err]);
        match result {
            Ok((child, w, r)) => {
                // SAFETY: the shell's pipe ends, owned from here by the files.
                let stdin = unsafe { std::fs::File::from_raw_handle(w as RawHandle) };
                // SAFETY: as above.
                let stdout = unsafe { std::fs::File::from_raw_handle(r as RawHandle) };
                Ok((child, stdin, stdout))
            }
            Err(e) => {
                close_all(&[shell_in, shell_out]);
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe's text names every attempt and the layers.
    #[test]
    fn probe_text_lists_attempts() {
        let report = Report {
            layers: vec!["a".into(), "b".into()],
            warnings: vec!["c: no".into()],
        };
        let text = probe_text(
            &report,
            &[
                Attempt {
                    what: "file-read",
                    outcome: Err("denied".into()),
                },
                Attempt {
                    what: "socket",
                    outcome: Ok(()),
                },
            ],
        );
        assert_eq!(
            text,
            "sandbox: a, b; not applied: c: no\nfile-read: refused (denied)\nsocket: allowed\n"
        );
        assert_eq!(lockdown(Mode::Off).warnings, ["VY_SANDBOX=off"]);
    }
}
