use crate::util::{process_error, read2b, CargoResult, CargoResultExt};
use anyhow::bail;
use jobserver::Client;
use shell_escape::escape;
use std::collections::BTreeMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::iter::once;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use log::debug;
use std::process;
use std::thread;

/// A builder object for an external process, similar to `std::process::Command`.
#[derive(Clone, Debug)]
pub struct ProcessBuilder {
    /// The program to execute.
    program: OsString,
    /// A list of arguments to pass to the program.
    args: Vec<OsString>,
    /// Any environment variables that should be set for the program.
    env: BTreeMap<String, Option<OsString>>,
    /// The directory to run the program from.
    cwd: Option<OsString>,
    /// The `make` jobserver. See the [jobserver crate][jobserver_docs] for
    /// more information.
    ///
    /// [jobserver_docs]: https://docs.rs/jobserver/0.1.6/jobserver/
    jobserver: Option<Client>,
    /// `true` to include environment variable in display.
    display_env_vars: bool,
}

impl fmt::Display for ProcessBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`")?;

        if self.display_env_vars {
            for (key, val) in self.env.iter() {
                if let Some(val) = val {
                    let val = escape(val.to_string_lossy());
                    if cfg!(windows) {
                        write!(f, "set {}={}&& ", key, val)?;
                    } else {
                        write!(f, "{}={} ", key, val)?;
                    }
                }
            }
        }

        write!(f, "{}", self.program.to_string_lossy())?;

        for arg in &self.args {
            write!(f, " {}", escape(arg.to_string_lossy()))?;
        }

        write!(f, "`")
    }
}

impl ProcessBuilder {
    /// (chainable) Sets the executable for the process.
    pub fn program<T: AsRef<OsStr>>(&mut self, program: T) -> &mut ProcessBuilder {
        self.program = program.as_ref().to_os_string();
        self
    }

    /// (chainable) Adds `arg` to the args list.
    pub fn arg<T: AsRef<OsStr>>(&mut self, arg: T) -> &mut ProcessBuilder {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    /// (chainable) Adds multiple `args` to the args list.
    pub fn args<T: AsRef<OsStr>>(&mut self, args: &[T]) -> &mut ProcessBuilder {
        self.args
            .extend(args.iter().map(|t| t.as_ref().to_os_string()));
        self
    }

    /// (chainable) Replaces the args list with the given `args`.
    pub fn args_replace<T: AsRef<OsStr>>(&mut self, args: &[T]) -> &mut ProcessBuilder {
        self.args = args.iter().map(|t| t.as_ref().to_os_string()).collect();
        self
    }

    /// (chainable) Sets the current working directory of the process.
    pub fn cwd<T: AsRef<OsStr>>(&mut self, path: T) -> &mut ProcessBuilder {
        self.cwd = Some(path.as_ref().to_os_string());
        self
    }

    /// (chainable) Sets an environment variable for the process.
    pub fn env<T: AsRef<OsStr>>(&mut self, key: &str, val: T) -> &mut ProcessBuilder {
        self.env
            .insert(key.to_string(), Some(val.as_ref().to_os_string()));
        self
    }

    /// (chainable) Unsets an environment variable for the process.
    pub fn env_remove(&mut self, key: &str) -> &mut ProcessBuilder {
        self.env.insert(key.to_string(), None);
        self
    }

    /// Gets the executable name.
    pub fn get_program(&self) -> &OsString {
        &self.program
    }

    /// Gets the program arguments.
    pub fn get_args(&self) -> &[OsString] {
        &self.args
    }

    /// Gets the current working directory for the process.
    pub fn get_cwd(&self) -> Option<&Path> {
        self.cwd.as_ref().map(Path::new)
    }

    /// Gets an environment variable as the process will see it (will inherit from environment
    /// unless explicitally unset).
    pub fn get_env(&self, var: &str) -> Option<OsString> {
        self.env
            .get(var)
            .cloned()
            .or_else(|| Some(env::var_os(var)))
            .and_then(|s| s)
    }

    /// Gets all environment variables explicitly set or unset for the process (not inherited
    /// vars).
    pub fn get_envs(&self) -> &BTreeMap<String, Option<OsString>> {
        &self.env
    }

    /// Sets the `make` jobserver. See the [jobserver crate][jobserver_docs] for
    /// more information.
    ///
    /// [jobserver_docs]: https://docs.rs/jobserver/0.1.6/jobserver/
    pub fn inherit_jobserver(&mut self, jobserver: &Client) -> &mut Self {
        self.jobserver = Some(jobserver.clone());
        self
    }

    /// Enables environment variable display.
    pub fn display_env_vars(&mut self) -> &mut Self {
        self.display_env_vars = true;
        self
    }

    /// Runs the process, waiting for completion, and mapping non-success exit codes to an error.
    pub fn exec(&self) -> CargoResult<()> {
        let mut command = self.build_command();
	debug!("lbt status() calling for {:?}", command);
        let exit = command.status().chain_err(|| {
            process_error(&format!("could not execute process {}", self), None, None)
        })?;
	debug!("lbt status() returned for {:?}", command);

        if exit.success() {
            Ok(())
        } else {
            Err(process_error(
                &format!("process didn't exit successfully: {}", self),
                Some(exit),
                None,
            )
            .into())
        }
    }

    /// Replaces the current process with the target process.
    ///
    /// On Unix, this executes the process using the Unix syscall `execvp`, which will block
    /// this process, and will only return if there is an error.
    ///
    /// On Windows this isn't technically possible. Instead we emulate it to the best of our
    /// ability. One aspect we fix here is that we specify a handler for the Ctrl-C handler.
    /// In doing so (and by effectively ignoring it) we should emulate proxying Ctrl-C
    /// handling to the application at hand, which will either terminate or handle it itself.
    /// According to Microsoft's documentation at
    /// <https://docs.microsoft.com/en-us/windows/console/ctrl-c-and-ctrl-break-signals>.
    /// the Ctrl-C signal is sent to all processes attached to a terminal, which should
    /// include our child process. If the child terminates then we'll reap them in Cargo
    /// pretty quickly, and if the child handles the signal then we won't terminate
    /// (and we shouldn't!) until the process itself later exits.
    pub fn exec_replace(&self) -> CargoResult<()> {
        imp::exec_replace(self)
    }

    /// Executes the process, returning the stdio output, or an error if non-zero exit status.
    pub fn exec_with_output(&self) -> CargoResult<Output> {
        let mut command = self.build_command();

	debug!("lbt (pid:{:?}/{:?}) about to exec_with_output: {:?}", process::id(), thread::current().id(), command);
        let output = command.output().chain_err(|| {
            process_error(&format!("could not execute process {}", self), None, None)
        })?;
	debug!("lbt (pid:{:?}/{:?}) done exec_with_output: {:?}", process::id(), thread::current().id(), command);

        if output.status.success() {
            Ok(output)
        } else {
            Err(process_error(
                &format!("process didn't exit successfully: {}", self),
                Some(output.status),
                Some(&output),
            )
            .into())
        }
    }

    /// Executes a command, passing each line of stdout and stderr to the supplied callbacks, which
    /// can mutate the string data.
    ///
    /// If any invocations of these function return an error, it will be propagated.
    ///
    /// If `capture_output` is true, then all the output will also be buffered
    /// and stored in the returned `Output` object. If it is false, no caching
    /// is done, and the callbacks are solely responsible for handling the
    /// output.
    pub fn exec_with_streaming(
        &self,
        on_stdout_line: &mut dyn FnMut(&str) -> CargoResult<()>,
        on_stderr_line: &mut dyn FnMut(&str) -> CargoResult<()>,
        capture_output: bool,
    ) -> CargoResult<Output> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        let mut cmd = self.build_command();
        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());

        let mut callback_error = None;
        let status = (|| {
            let mut child = cmd.spawn()?;
	    let child_id = child.id();
	    debug!("lbt (pid:{:?}/{:?} Spawned pid:{:?} for {:?} capture: {}", process::id(), thread::current().id(), child_id, cmd, capture_output);
            let out = child.stdout.take().unwrap();
            let err = child.stderr.take().unwrap();
	    //read2(out, err, &mut |is_out, data, eof| {
            read2b(out, err, &mut child, &mut |is_out, data, eof| {
		debug!("lbt (pid:{:?}/{:?}) Got some {} read2b from child {}. Eof={}", process::id(), thread::current().id(), if is_out {"out"} else {"err"}, child_id, eof);
                let idx = if eof {
                    data.len()
                } else {
                    match data.iter().rposition(|b| *b == b'\n') {
                        Some(i) => i + 1,
                        None => return,
                    }
                };
		debug!("lbt (pid:{:?}/{:?}) idx {}", process::id(), thread::current().id(), idx);
                {
                    // scope for new_lines
                    let new_lines = if capture_output {
                        let dst = if is_out { &mut stdout } else { &mut stderr };
                        let start = dst.len();
                        let data = data.drain(..idx);
                        dst.extend(data);
                        &dst[start..]
                    } else {
                        &data[..idx]
                    };
                    for line in String::from_utf8_lossy(new_lines).lines() {
                        if callback_error.is_some() {
			    debug!("lbt (pid:{:?}/{:?}) callback error set, not trying any more", process::id(), thread::current().id());
                            break;
                        }
                        let callback_result = if is_out {
			    debug!("lbt (pid:{:?}/{:?}) stdout callback from child {}", process::id(), thread::current().id(), child_id);
                            let res = on_stdout_line(line);
			    debug!("lbt (pid:{:?}/{:?}) stdout callback from child {} done", process::id(), thread::current().id(), child_id);
			    res
                        } else {
			    debug!("lbt (pid:{:?}/{:?}) stderr callback from child {}", process::id(), thread::current().id(), child_id);
                            let res = on_stderr_line(line);
			    debug!("lbt (pid:{:?}/{:?}) stderr callback from child {} done", process::id(), thread::current().id(), child_id);
			    res
                        };
                        if let Err(e) = callback_result {
			    debug!("lbt (pid:{:?}/{:?}) from child {} callback error: {}", process::id(), thread::current().id(), child_id, e);
                            callback_error = Some(e);
                        }
                    }
                }
                if !capture_output {
		    debug!("lbt (pid:{:?}/{:?}) draining child {}", process::id(), thread::current().id(), child_id);
                    data.drain(..idx);
		    debug!("lbt (pid:{:?}/{:?}) drained child {}", process::id(), thread::current().id(), child_id);
                }
            })?;  // this is the call to read2... note the ?
	    debug!("lbt (pid:{:?}/{:?}) Waiting for pid:{:?}", process::id(), thread::current().id(), child_id);
	    //child.wait()
            let res = child.wait();
	    debug!("lbt (pid:{:?}/{:?}) Waited for pid:{:?}", process::id(), thread::current().id(), child_id);
	    res
	    //match resr2 {
 	    // 	Ok(_b) => res,
	    // 	Err(e) => {
	    // 	    debug!("lbt (pid:{:?}/{:?}) Caught error in read2", process::id(), thread::current().id());
	    // 	    Err(e) },
	    // }
        })()
        .chain_err(|| process_error(&format!("could not execute process {}", self), None, None))?;
        let output = Output {
            stdout,
            stderr,
            status,
        };

        {
            let to_print = if capture_output { Some(&output) } else { None };
            if let Some(e) = callback_error {
                let cx = process_error(
                    &format!("failed to parse process output: {}", self),
                    Some(output.status),
                    to_print,
                );
                bail!(anyhow::Error::new(cx).context(e));
            } else if !output.status.success() {
                bail!(process_error(
                    &format!("process didn't exit successfully: {}", self),
                    Some(output.status),
                    to_print,
                ));
            }
        }

        Ok(output)
    }

    /// Converts `ProcessBuilder` into a `std::process::Command`, and handles the jobserver, if
    /// present.
    pub fn build_command(&self) -> Command {
        let mut command = Command::new(&self.program);
        if let Some(cwd) = self.get_cwd() {
            command.current_dir(cwd);
        }
        for arg in &self.args {
            command.arg(arg);
        }
        for (k, v) in &self.env {
            match *v {
                Some(ref v) => {
                    command.env(k, v);
                }
                None => {
                    command.env_remove(k);
                }
            }
        }
        if let Some(ref c) = self.jobserver {
            c.configure(&mut command);
        }
        command
    }

    /// Wraps an existing command with the provided wrapper, if it is present and valid.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use cargo::util::{ProcessBuilder, process};
    /// // Running this would execute `rustc`
    /// let cmd: ProcessBuilder = process("rustc");
    ///
    /// // Running this will execute `sccache rustc`
    /// let cmd = cmd.wrapped(Some("sccache"));
    /// ```
    pub fn wrapped(mut self, wrapper: Option<impl AsRef<OsStr>>) -> Self {
        let wrapper = if let Some(wrapper) = wrapper.as_ref() {
            wrapper.as_ref()
        } else {
            return self;
        };

        if wrapper.is_empty() {
            return self;
        }

        let args = once(self.program).chain(self.args.into_iter()).collect();

        self.program = wrapper.to_os_string();
        self.args = args;

        self
    }
}

/// A helper function to create a `ProcessBuilder`.
pub fn process<T: AsRef<OsStr>>(cmd: T) -> ProcessBuilder {
    ProcessBuilder {
        program: cmd.as_ref().to_os_string(),
        args: Vec::new(),
        cwd: None,
        env: BTreeMap::new(),
        jobserver: None,
        display_env_vars: false,
    }
}

#[cfg(unix)]
mod imp {
    use crate::util::{process_error, ProcessBuilder};
    use crate::CargoResult;
    use std::os::unix::process::CommandExt;
    use log::debug;
    
    pub fn exec_replace(process_builder: &ProcessBuilder) -> CargoResult<()> {
        let mut command = process_builder.build_command();
	debug!("About to execvp {:?}", process_builder);
        let error = command.exec();
        Err(anyhow::Error::from(error)
            .context(process_error(
                &format!("could not execute process {}", process_builder),
                None,
                None,
            ))
            .into())
    }
}

#[cfg(windows)]
mod imp {
    use crate::util::{process_error, ProcessBuilder};
    use crate::CargoResult;
    use winapi::shared::minwindef::{BOOL, DWORD, FALSE, TRUE};
    use winapi::um::consoleapi::SetConsoleCtrlHandler;

    unsafe extern "system" fn ctrlc_handler(_: DWORD) -> BOOL {
        // Do nothing; let the child process handle it.
        TRUE
    }

    pub fn exec_replace(process_builder: &ProcessBuilder) -> CargoResult<()> {
        unsafe {
            if SetConsoleCtrlHandler(Some(ctrlc_handler), TRUE) == FALSE {
                return Err(process_error("Could not set Ctrl-C handler.", None, None).into());
            }
        }

        // Just execute the process as normal.
        process_builder.exec()
    }
}
