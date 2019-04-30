extern crate libc;

use std::error;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::io::{Error, ErrorKind};
use std::mem;
use std::ptr::NonNull;
use std::time::{SystemTime, UNIX_EPOCH};

use libc::{c_int, sem_t};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct NoToken;

impl Display for NoToken {
    fn fmt(&self, fmt: &mut Formatter) -> FmtResult {
        write!(fmt, "No token available")
    }
}

impl error::Error for NoToken {}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct Overflow;

impl Display for Overflow {
    fn fmt(&self, fmt: &mut Formatter) -> FmtResult {
        write!(fmt, "Overflow of a semaphore")
    }
}

impl error::Error for Overflow {}

enum Mode {
    Uninitialized,
    Anonymous,
}

pub struct Semaphore {
    inner: NonNull<sem_t>,
    mode: Mode,
}

impl Semaphore {
    unsafe fn uninitialized() -> Self {
        let inner = Box::into_raw(Box::new(mem::zeroed()));
        let inner = NonNull::new(inner).unwrap();

        Semaphore {
            inner,
            mode: Mode::Uninitialized,
        }
    }

    pub fn anonymous(value: c_int) -> Result<Self, Error> {
        unsafe {
            let mut me = Self::uninitialized();

            match libc::sem_init(me.inner.as_ptr(), 0, value as _) {
                0 => {
                    me.mode = Mode::Anonymous;
                    Ok(me)
                },
                // Note: the destructor will take care of disposing of the memory, etc.
                -1 => Err(Error::last_os_error()),
                other => unreachable!("sem_init doesn't return value {}", other),
            }
        }
    }

    pub fn wait(&self) {
        unsafe {
            loop {
                if libc::sem_wait(self.inner.as_ptr()) == 0 {
                    return;
                } else {
                    let e = Error::last_os_error();
                    assert!(e.kind() == ErrorKind::Interrupted);
                }
            }
        }
    }

    pub fn trywait(&self) -> Result<(), NoToken> {
        unsafe {
            loop {
                if libc::sem_trywait(self.inner.as_ptr()) == 0 {
                    return Ok(())
                } else {
                    let e = Error::last_os_error();
                    match e.kind() {
                        ErrorKind::Interrupted => continue,
                        ErrorKind::WouldBlock => return Err(NoToken),
                        _ => unreachable!("Impossible error {}", e),
                    }
                }
            }
        }
    }

    pub fn timedwait(&self, until: SystemTime) -> Result<(), NoToken> {
        let dur = until.duration_since(UNIX_EPOCH).unwrap();
        let timespec = libc::timespec {
            tv_sec: dur.as_secs() as _,
            tv_nsec: i64::from(dur.subsec_nanos()),
        };

        unsafe {
            loop {
                if libc::sem_timedwait(self.inner.as_ptr(), &timespec) == 0 {
                    return Ok(())
                } else {
                    let e = Error::last_os_error();
                    match e.kind() {
                        ErrorKind::Interrupted => continue,
                        ErrorKind::TimedOut => return Err(NoToken),
                        _ => unreachable!("Impossible error {}", e),
                    }
                }
            }
        }
    }

    pub fn post(&self) -> Result<(), Overflow> {
        unsafe {
            if libc::sem_post(self.inner.as_ptr()) == 0 {
                Ok(())
            } else if Error::last_os_error().raw_os_error() == Some(libc::EOVERFLOW) {
                Err(Overflow)
            } else {
                unreachable!("Semaphore corruption")
            }
        }
    }

    pub fn value(&self) -> c_int {
        unsafe {
            let mut val = 0;
            assert_eq!(0, libc::sem_getvalue(self.inner.as_ptr(), &mut val));
            val
        }
    }
}

impl Drop for Semaphore {
    fn drop(&mut self) {
        unsafe {
            match self.mode {
                Mode::Uninitialized => (),
                Mode::Anonymous => {
                    assert_eq!(0, libc::sem_destroy(self.inner.as_ptr()), "Corrupt semaphore");
                }
            }

            drop(Box::from_raw(self.inner.as_ptr()));
        }
    }
}

unsafe impl Send for Semaphore {}
unsafe impl Sync for Semaphore {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::*;

    #[test]
    fn anon_create_destroy() {
        let sem = Semaphore::anonymous(0).unwrap();
        drop(sem);
    }

    #[test]
    fn wait_one() {
        let sem = Semaphore::anonymous(1).unwrap();
        assert_eq!(1, sem.value());
        sem.wait();
        assert_eq!(0, sem.value());
        sem.post().unwrap();
        assert_eq!(1, sem.value());
    }

    #[test]
    fn wait_thread() {
        let sem = Arc::new(Semaphore::anonymous(0).unwrap());
        thread::spawn({
            let sem = Arc::clone(&sem);
            move || {
                sem.post().unwrap();
                sem.post().unwrap();
            }
        });
        sem.wait();
        sem.wait();
    }

    #[test]
    fn try_wait() {
        let sem = Semaphore::anonymous(0).unwrap();
        sem.trywait().unwrap_err();
        sem.post().unwrap();
        sem.trywait().unwrap();
    }
}
