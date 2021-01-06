pub use self::imp::read2;
pub use self::imp::read2a;
pub use self::imp::read2b;

#[cfg(unix)]
mod imp {
    use std::io;
    use std::io::prelude::*;
    use std::mem;
    use std::os::unix::prelude::*;
    use std::process::{Child, ChildStderr, ChildStdout};
    use std::process;
    use log::debug;

    pub fn read2a(
        mut out_pipe: ChildStdout,
        mut err_pipe: ChildStderr,
        child: &mut Child,
        data: &mut dyn FnMut(bool, &mut Vec<u8>, bool),
    ) -> io::Result<()> {
        unsafe {
            libc::fcntl(out_pipe.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK);
            libc::fcntl(err_pipe.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK);
        }

        let mut out_done = false;
        let mut err_done = false;
        let mut out = Vec::new();
        let mut err = Vec::new();

        let mut fds: [libc::pollfd; 2] = unsafe { mem::zeroed() };
        fds[0].fd = out_pipe.as_raw_fd();
        fds[0].events = libc::POLLIN;
        fds[1].fd = err_pipe.as_raw_fd();
        fds[1].events = libc::POLLIN;
        let mut nfds = 2;
        let mut errfd = 1;
	let mut child_is_alive: bool = match child.try_wait() {
	    Ok(Some(_status)) => false,
	    Ok(None) => true,
	    Err(e) => { debug!("lbt (pid:{}) read2a error attempting to pre-wait for {} : {}",
			       process::id(), child.id(), e);
			false }
	};

        while nfds > 0 && child_is_alive {
            // wait for either pipe to become readable using `select`
            let r = unsafe {
		let mut rr;
		while {rr = libc::poll(fds.as_mut_ptr(), nfds, 500); rr == 0} {
		    child_is_alive = match child.try_wait() {
			Ok(Some(_status)) => false,
			Ok(None) => true,
			Err(e) => {
			    debug!("lbt (pid:{}) read2a error attempting to wait for {} : {}",
				   process::id(), child.id(), e);
			    return Err(e);
			}
		    };
		    debug!("lbt (pid:{}) read2a poll timout and child pid:{} is{} alive",
			   process::id(), child.id(),
			   if child_is_alive {""} else {" no longer"});
		    if !child_is_alive {
			return Ok(()); // same as nfds == 0
		    }
		}
		rr
	    };
            if r == -1 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }

            // Read as much as we can from each pipe, ignoring EWOULDBLOCK or
            // EAGAIN. If we hit EOF, then this will happen because the underlying
            // reader will return Ok(0), in which case we'll see `Ok` ourselves. In
            // this case we flip the other fd back into blocking mode and read
            // whatever's leftover on that file descriptor.
            let handle = |res: io::Result<_>| match res {
                Ok(_) => Ok(true),
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        Ok(false)
                    } else {
			// This does return because the handle call below is ?'ed
			debug!("lbt (pid:{}) read2a returns Err while draining", process::id());
                        Err(e)
                    }
                }
            };
            if !err_done && fds[errfd].revents != 0 && handle(err_pipe.read_to_end(&mut err))? {
                err_done = true;
                nfds -= 1;
            }
            data(false, &mut err, err_done);
            if !out_done && fds[0].revents != 0 && handle(out_pipe.read_to_end(&mut out))? {
                out_done = true;
                fds[0].fd = err_pipe.as_raw_fd();
                errfd = 0;
                nfds -= 1;
            }
            data(true, &mut out, out_done);
        }
	debug!("lbt (pid:{}) read2a returns OK", process::id());
        Ok(())
    }
    pub fn read2b(
        mut out_pipe: ChildStdout,
        mut err_pipe: ChildStderr,
        child: &mut Child,
        data: &mut dyn FnMut(bool, &mut Vec<u8>, bool),
    ) -> io::Result<()> {
        unsafe {
            libc::fcntl(out_pipe.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK);
            libc::fcntl(err_pipe.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK);
        }

        let mut out_done = false;
        let mut err_done = false;
        let mut out = Vec::new();
        let mut err = Vec::new();

        let mut fds: [libc::pollfd; 2] = unsafe { mem::zeroed() };
        fds[0].fd = out_pipe.as_raw_fd();
        fds[0].events = libc::POLLIN;
        fds[1].fd = err_pipe.as_raw_fd();
        fds[1].events = libc::POLLIN;
        let mut nfds = 2;
        let mut errfd = 1;
	let child_id = child.id();
	let mut _child_is_alive: bool = match child.try_wait() {
	    Ok(Some(_status)) => false,
	    Ok(None) => true,
	    Err(e) => { debug!("lbt (pid:{}) read2b error attempting to pre-wait() for {} : {}",
			       process::id(), child_id, e);
			false }
	};

        while nfds > 0 {
            // wait for either pipe to become readable using `select`
 	    debug!("lbt (pid:{}) waiting for read2b poll for {}", process::id(), child_id);
            let r = unsafe {
		let mut rr;
		while {rr = libc::poll(fds.as_mut_ptr(), nfds, 500); rr == 0} {
		    debug!("lbt (pid:{}) read2b polling timout for child pid:{}",
			   process::id(), child_id);
		}
		rr
	    };
            if r == -1 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
		debug!("lbt (pid:{}) read2b returns Err for child {}", process::id(), child_id);
                return Err(err);
            }

            // Read as much as we can from each pipe, ignoring EWOULDBLOCK or
            // EAGAIN. If we hit EOF, then this will happen because the underlying
            // reader will return Ok(0), in which case we'll see `Ok` ourselves. In
            // this case we flip the other fd back into blocking mode and read
            // whatever's leftover on that file descriptor.
            let handle = |res: io::Result<_>| match res {
                Ok(_) => Ok(true),
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        Ok(false)
                    } else {
			// This does return because the handle call below is ?'ed
			debug!("lbt (pid:{}) read2b returns Err for child {} while draining", process::id(), child_id);
                        Err(e)
                    }
                }
            };
            if !err_done && fds[errfd].revents != 0 && handle(err_pipe.read_to_end(&mut err))? {
                err_done = true;
                nfds -= 1;
            }
            data(false, &mut err, err_done);
            if !out_done && fds[0].revents != 0 && handle(out_pipe.read_to_end(&mut out))? {
                out_done = true;
                fds[0].fd = err_pipe.as_raw_fd();
                errfd = 0;
                nfds -= 1;
            }
            data(true, &mut out, out_done);
    }
    debug!("lbt (pid:{}) read2b returns OK for child {}", process::id(), child_id);
    Ok(())
}
    pub fn read2(
        mut out_pipe: ChildStdout,
        mut err_pipe: ChildStderr,
        data: &mut dyn FnMut(bool, &mut Vec<u8>, bool),
    ) -> io::Result<()> {
        unsafe {
            libc::fcntl(out_pipe.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK);
            libc::fcntl(err_pipe.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK);
        }

        let mut out_done = false;
        let mut err_done = false;
        let mut out = Vec::new();
        let mut err = Vec::new();

        let mut fds: [libc::pollfd; 2] = unsafe { mem::zeroed() };
        fds[0].fd = out_pipe.as_raw_fd();
        fds[0].events = libc::POLLIN;
        fds[1].fd = err_pipe.as_raw_fd();
        fds[1].events = libc::POLLIN;
        let mut nfds = 2;
        let mut errfd = 1;

        while nfds > 0 {
            // wait for either pipe to become readable using `select`
	    debug!("lbt (pid:{}) wait for read2 poll", process::id());
            let r = unsafe { libc::poll(fds.as_mut_ptr(), nfds, -1) };
            if r == -1 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
		debug!("lbt (pid:{}) read2 returns Err", process::id());
                return Err(err);
            }

            // Read as much as we can from each pipe, ignoring EWOULDBLOCK or
            // EAGAIN. If we hit EOF, then this will happen because the underlying
            // reader will return Ok(0), in which case we'll see `Ok` ourselves. In
            // this case we flip the other fd back into blocking mode and read
            // whatever's leftover on that file descriptor.
            let handle = |res: io::Result<_>| match res {
                Ok(_) => Ok(true),
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        Ok(false)
                    } else {
			// This does return because the handle call below is ?'ed
			debug!("lbt (pid:{}) read2 returns Err while draining", process::id());
                        Err(e)
                    }
                }
            };
            if !err_done && fds[errfd].revents != 0 && handle(err_pipe.read_to_end(&mut err))? {
                err_done = true;
                nfds -= 1;
            }
            data(false, &mut err, err_done);
            if !out_done && fds[0].revents != 0 && handle(out_pipe.read_to_end(&mut out))? {
                out_done = true;
                fds[0].fd = err_pipe.as_raw_fd();
                errfd = 0;
                nfds -= 1;
            }
            data(true, &mut out, out_done);
        }
	debug!("lbt (pid:{}) read2 returns OK", process::id());
        Ok(())
    }
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::os::windows::prelude::*;
    use std::process::{ChildStderr, ChildStdout};
    use std::slice;

    use miow::iocp::{CompletionPort, CompletionStatus};
    use miow::pipe::NamedPipe;
    use miow::Overlapped;
    use winapi::shared::winerror::ERROR_BROKEN_PIPE;

    struct Pipe<'a> {
        dst: &'a mut Vec<u8>,
        overlapped: Overlapped,
        pipe: NamedPipe,
        done: bool,
    }

    pub fn read2(
        out_pipe: ChildStdout,
        err_pipe: ChildStderr,
        data: &mut dyn FnMut(bool, &mut Vec<u8>, bool),
    ) -> io::Result<()> {
        let mut out = Vec::new();
        let mut err = Vec::new();

        let port = CompletionPort::new(1)?;
        port.add_handle(0, &out_pipe)?;
        port.add_handle(1, &err_pipe)?;

        unsafe {
            let mut out_pipe = Pipe::new(out_pipe, &mut out);
            let mut err_pipe = Pipe::new(err_pipe, &mut err);

            out_pipe.read()?;
            err_pipe.read()?;

            let mut status = [CompletionStatus::zero(), CompletionStatus::zero()];

            while !out_pipe.done || !err_pipe.done {
                for status in port.get_many(&mut status, None)? {
                    if status.token() == 0 {
                        out_pipe.complete(status);
                        data(true, out_pipe.dst, out_pipe.done);
                        out_pipe.read()?;
                    } else {
                        err_pipe.complete(status);
                        data(false, err_pipe.dst, err_pipe.done);
                        err_pipe.read()?;
                    }
                }
            }

            Ok(())
        }
    }

    impl<'a> Pipe<'a> {
        unsafe fn new<P: IntoRawHandle>(p: P, dst: &'a mut Vec<u8>) -> Pipe<'a> {
            Pipe {
                dst,
                pipe: NamedPipe::from_raw_handle(p.into_raw_handle()),
                overlapped: Overlapped::zero(),
                done: false,
            }
        }

        unsafe fn read(&mut self) -> io::Result<()> {
            let dst = slice_to_end(self.dst);
            match self.pipe.read_overlapped(dst, self.overlapped.raw()) {
                Ok(_) => Ok(()),
                Err(e) => {
                    if e.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
                        self.done = true;
                        Ok(())
                    } else {
                        Err(e)
                    }
                }
            }
        }

        unsafe fn complete(&mut self, status: &CompletionStatus) {
            let prev = self.dst.len();
            self.dst.set_len(prev + status.bytes_transferred() as usize);
            if status.bytes_transferred() == 0 {
                self.done = true;
            }
        }
    }

    unsafe fn slice_to_end(v: &mut Vec<u8>) -> &mut [u8] {
        if v.capacity() == 0 {
            v.reserve(16);
        }
        if v.capacity() == v.len() {
            v.reserve(1);
        }
        slice::from_raw_parts_mut(v.as_mut_ptr().add(v.len()), v.capacity() - v.len())
    }
}
