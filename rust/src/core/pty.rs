use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::libc::{self, TIOCSCTTY, TIOCSWINSZ, winsize};
use nix::pty::{OpenptyResult, openpty};
use nix::sys::signal::{Signal, kill};
use nix::unistd::{ForkResult, Pid, execvp, fork, setsid};
use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};

pub struct Pty {
    master: OwnedFd,
    child_pid: Pid,
}

impl Pty {
    pub fn spawn(shell: &str, rows: u16, cols: u16) -> io::Result<Self> {
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

                unsafe {
                    let data_dir = CString::new("/data/local/tmp").unwrap();
                    libc::chdir(data_dir.as_ptr());
                }

                unsafe {
                    std::env::set_var("TERM", "xterm-256color");
                    std::env::set_var("HOME", "/data/local/tmp");
                    std::env::set_var("PATH", "/system/bin:/system/xbin");
                }

                let shell_cstr = CString::new(shell).unwrap();
                let args = [shell_cstr.clone()];
                execvp(&shell_cstr, &args).ok();

                std::process::exit(1);
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

impl Drop for Pty {
    fn drop(&mut self) {
        let _ = kill(self.child_pid, Signal::SIGHUP);
    }
}
