use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::libc::{self, winsize, TIOCSCTTY, TIOCSWINSZ};
use nix::pty::{openpty, OpenptyResult};
use nix::sys::signal::{kill, Signal};
use nix::unistd::{execv, fork, setsid, ForkResult, Pid};
use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub struct Pty {
    master: OwnedFd,
    child_pid: Pid,
}

impl Pty {
    pub fn spawn(shell: &str, rows: u16, cols: u16, env: &PtyEnv) -> io::Result<Self> {
        let OpenptyResult { master, slave } =
            openpty(None, None).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        let ws = winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            libc::ioctl(master.as_raw_fd(), TIOCSWINSZ, &ws);
        }

        match unsafe { fork() } {
            Ok(ForkResult::Parent { child }) => {
                drop(slave);
                let flags = fcntl(&master, FcntlArg::F_GETFL)
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
                let flags = OFlag::from_bits_truncate(flags);
                fcntl(&master, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

                log::info!(
                    "PTY spawned: child={}, master_fd={}",
                    child,
                    master.as_raw_fd()
                );
                Ok(Pty {
                    master,
                    child_pid: child,
                })
            }
            Ok(ForkResult::Child) => {
                drop(master);
                setsid().ok();
                unsafe {
                    libc::ioctl(slave.as_raw_fd(), TIOCSCTTY, 0);
                }

                let slave_fd = slave.as_raw_fd();
                unsafe {
                    libc::dup2(slave_fd, 0); // stdin
                    libc::dup2(slave_fd, 1); // stdout
                    libc::dup2(slave_fd, 2); // stderr
                }

                if slave_fd > 2 {
                    drop(slave);
                }

                if let Some(dir) = env.cwd.as_ref() {
                    if let Ok(cwd) = CString::new(dir.as_os_str().as_bytes()) {
                        log::info!("PTY chdir to {:?}", dir);
                        unsafe {
                            libc::chdir(cwd.as_ptr());
                        }
                    }
                }

                log::info!("PTY env TERM={}", env.term);
                log::info!("PTY env HOME={:?}", env.home);
                log::info!("PTY env PATH={}", env.path);
                if let Some(ref tmp) = env.tmp {
                    log::info!("PTY env TMPDIR={:?}", tmp);
                }
                if let Some(ref prefix) = env.prefix {
                    log::info!("PTY env PREFIX={:?}", prefix);
                }
                if let Some(ref ld) = env.ld_library_path {
                    log::info!("PTY env LD_LIBRARY_PATH={}", ld);
                }
                if let Some(ref preload) = env.ld_preload {
                    log::info!("PTY env LD_PRELOAD={}", preload);
                }

                let term = select_term_for_env(env);
                if term != env.term {
                    log::warn!(
                        "TERM '{}' not available, falling back to '{}'",
                        env.term,
                        term
                    );
                }

                unsafe {
                    std::env::set_var("TERM", term.as_str());
                    std::env::set_var("HOME", env.home.as_os_str());
                    std::env::set_var("PATH", env.path.as_str());
                    std::env::set_var("SHELL", shell);
                    if let Some(ref tmp) = env.tmp {
                        std::env::set_var("TMPDIR", tmp.as_os_str());
                    }
                    if let Some(ref prefix) = env.prefix {
                        std::env::set_var("PREFIX", prefix.as_os_str());
                        std::env::set_var("TERMUX_PREFIX", prefix.as_os_str());
                        std::env::set_var("TERMUX__ROOTFS", prefix.as_os_str());
                        std::env::set_var("TERMUX_ANDROID10", "1");
                        std::env::set_var("TERMUX_EXEC__SYSTEM_LINKER_EXEC", "enable");
                        std::env::set_var("DPKG_ROOT", prefix.as_os_str());
                        std::env::set_var("DPKG_ADMINDIR", prefix.join("var/lib/dpkg").as_os_str());
                        std::env::set_var(
                            "APT_CONFIG",
                            prefix.join("etc/apt/apt.conf").as_os_str(),
                        );
                        let ca_cert = prefix.join("etc/tls/cert.pem");
                        std::env::set_var("SSL_CERT_FILE", ca_cert.as_os_str());
                        std::env::set_var("CURL_CA_BUNDLE", ca_cert.as_os_str());
                        std::env::set_var("GIT_SSL_CAINFO", ca_cert.as_os_str());
                        std::env::set_var("REQUESTS_CA_BUNDLE", ca_cert.as_os_str());
                        std::env::set_var("NODE_EXTRA_CA_CERTS", ca_cert.as_os_str());
                        let ca_dir = prefix.join("etc/tls/certs");
                        std::env::set_var("SSL_CERT_DIR", ca_dir.as_os_str());
                        let terminfo = prefix.join("share/terminfo");
                        let terminfo_lib = prefix.join("lib/terminfo");
                        let terminfo_dirs =
                            format!("{}:{}", terminfo.display(), terminfo_lib.display());
                        std::env::set_var("TERMINFO", terminfo.as_os_str());
                        std::env::set_var("TERMINFO_DIRS", terminfo_dirs);
                    }
                    if let Some(ref ld) = env.ld_library_path {
                        std::env::set_var("LD_LIBRARY_PATH", ld.as_str());
                    }
                    if let Some(ref preload) = env.ld_preload {
                        std::env::set_var("LD_PRELOAD", preload.as_str());
                    } else {
                        std::env::remove_var("LD_PRELOAD");
                    }
                }

                let shell_cstr = match CString::new(shell) {
                    Ok(s) => s,
                    Err(_) => {
                        log::error!("Shell path contains NUL byte: {:?}", shell);
                        std::process::exit(127);
                    }
                };

                let exec_result = if should_use_system_linker_exec(shell) {
                    if env.ld_preload.is_none() {
                        unsafe {
                            std::env::remove_var("LD_PRELOAD");
                        }
                    }

                    let linker = select_system_linker();
                    let linker_cstr = match CString::new(linker) {
                        Ok(s) => s,
                        Err(_) => {
                            log::error!("System linker path contains NUL byte: {}", linker);
                            std::process::exit(127);
                        }
                    };

                    log::info!(
                        "Executing via system linker: linker={}, target={}",
                        linker,
                        shell
                    );
                    let args = [linker_cstr.as_c_str(), shell_cstr.as_c_str()];
                    execv(linker_cstr.as_c_str(), &args)
                } else {
                    let args = [shell_cstr.as_c_str()];
                    execv(shell_cstr.as_c_str(), &args)
                };

                let e = exec_result.expect_err("execv unexpectedly returned success");
                log::error!("exec failed for {}: {:?}", shell, e);

                std::process::exit(127);
            }
            Err(e) => Err(io::Error::new(io::ErrorKind::Other, e)),
        }
    }

    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let n = unsafe {
            libc::read(
                self.master.as_raw_fd(),
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
            )
        };

        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::WouldBlock {
                Ok(0)
            } else {
                Err(err)
            }
        } else {
            Ok(n as usize)
        }
    }

    pub fn write(&self, data: &[u8]) -> io::Result<usize> {
        let n = unsafe {
            libc::write(
                self.master.as_raw_fd(),
                data.as_ptr() as *const libc::c_void,
                data.len(),
            )
        };

        if n < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(n as usize)
        }
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        let ws = winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            libc::ioctl(self.master.as_raw_fd(), TIOCSWINSZ, &ws);
        }
        let _ = kill(self.child_pid, Signal::SIGWINCH);
    }

    pub fn master_fd(&self) -> RawFd {
        self.master.as_raw_fd()
    }

    pub fn child_pid(&self) -> Pid {
        self.child_pid
    }
}

#[derive(Clone)]
pub struct PtyEnv {
    pub term: String,
    pub home: std::path::PathBuf,
    pub cwd: Option<std::path::PathBuf>,
    pub path: String,
    pub tmp: Option<std::path::PathBuf>,
    pub prefix: Option<std::path::PathBuf>,
    pub ld_library_path: Option<String>,
    pub ld_preload: Option<String>,
}

impl PtyEnv {
    pub fn system_default() -> Self {
        Self {
            term: "xterm-256color".to_string(),
            home: Path::new("/data/local/tmp").to_path_buf(),
            cwd: Some(Path::new("/data/local/tmp").to_path_buf()),
            path: "/system/bin:/system/xbin".to_string(),
            tmp: None,
            prefix: None,
            ld_library_path: None,
            ld_preload: None,
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        let _ = kill(self.child_pid, Signal::SIGHUP);
    }
}

fn should_use_system_linker_exec(target: &str) -> bool {
    target.starts_with("/data/") || target.starts_with("/mnt/expand/")
}

fn select_system_linker() -> &'static str {
    const LINKER64: &str = "/system/bin/linker64";
    const LINKER32: &str = "/system/bin/linker";

    if cfg!(target_pointer_width = "64") && Path::new(LINKER64).exists() {
        return LINKER64;
    }
    if Path::new(LINKER32).exists() {
        return LINKER32;
    }
    LINKER64
}

fn select_term_for_env(env: &PtyEnv) -> String {
    let requested = env.term.as_str();

    if let Some(prefix) = env.prefix.as_ref() {
        if terminfo_entry_exists(prefix, requested) {
            return requested.to_string();
        }
        if requested == "xterm-256color" && terminfo_entry_exists(prefix, "xterm") {
            return "xterm".to_string();
        }
    }

    requested.to_string()
}

fn terminfo_entry_exists(prefix: &Path, term: &str) -> bool {
    let Some(first_char) = term.chars().next() else {
        return false;
    };
    let first = first_char.to_string();

    let share_entry = prefix.join("share/terminfo").join(&first).join(term);
    if share_entry.is_file() {
        return true;
    }

    let lib_entry = prefix.join("lib/terminfo").join(&first).join(term);
    lib_entry.is_file()
}
