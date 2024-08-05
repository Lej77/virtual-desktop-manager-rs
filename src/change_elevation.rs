use deelevate::{Command, PrivilegeLevel, Token};
use std::{
    any::Any,
    borrow::Cow,
    ffi::OsString,
    io::{Read, Result as IoResult},
    net::{Shutdown, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, TryRecvError},
        OnceLock,
    },
    time::Duration,
};

/// Workaround for the `deelevate::process::Process` type that is private.
#[allow(clippy::type_complexity)]
struct Process<T = Box<dyn Any>> {
    state: T,
    /// Wait for the specified duration (in milliseconds!) to pass.
    /// Use None to wait forever.
    wait_for: Box<dyn Fn(&T, Option<u32>) -> IoResult<u32>>,
    /// Retrieves the exit code from the process
    exit_code: Box<dyn Fn(&T) -> IoResult<u32>>,
}
impl<T> Process<T> {
    fn into_any(self) -> Process
    where
        T: Any,
    {
        Process {
            state: Box::new(self.state),
            wait_for: Box::new(move |s, duration| {
                (self.wait_for)(s.downcast_ref().unwrap(), duration)
            }),
            exit_code: Box::new(move |s| (self.exit_code)(s.downcast_ref().unwrap())),
        }
    }

    /// Wait for the specified duration (in milliseconds!) to pass.
    /// Use None to wait forever.
    pub fn wait_for(&self, duration: Option<u32>) -> IoResult<u32> {
        (self.wait_for)(&self.state, duration)
    }

    /// Retrieves the exit code from the process
    pub fn exit_code(&self) -> IoResult<u32> {
        (self.exit_code)(&self.state)
    }
}
macro_rules! into_process {
    ($expr:expr) => {
        Process {
            state: $expr,
            wait_for: Box::new(|s, dur| s.wait_for(dur)),
            exit_code: Box::new(|s| s.exit_code()),
        }
        .into_any()
    };
}

pub trait SetElevationHandler: Send {
    fn get_args(&mut self, port: u16) -> Vec<OsString>;
    fn exit(&mut self) -> !;
    fn confirm_message(&mut self) -> Cow<'_, [u8]>;
}

pub fn set_elevation(
    app: &mut dyn SetElevationHandler,
    should_elevate: bool,
) -> Result<(), String> {
    let token = Token::with_current_process()
        .map_err(|e| format!("failed to get token for current process: {e}"))?;
    let level = token
        .privilege_level()
        .map_err(|e| format!("failed to get privilege level of token: {e}"))?;

    let target_token = if should_elevate {
        match level {
            PrivilegeLevel::NotPrivileged => token
                .as_medium_integrity_safer_token()
                .map_err(|e| format!("failed to create token with elevated privilege: {e}"))?,
            PrivilegeLevel::HighIntegrityAdmin | PrivilegeLevel::Elevated => return Ok(()),
        }
    } else {
        match level {
            PrivilegeLevel::NotPrivileged => return Ok(()),
            PrivilegeLevel::Elevated => Token::with_shell_process()
                .map_err(|e| format!("failed to find token for shell process: {e}"))?,
            PrivilegeLevel::HighIntegrityAdmin => token
                .as_medium_integrity_safer_token()
                .map_err(|e| format!("failed to change privilege level of token: {e}"))?,
        }
    };

    let mut command = Command::with_environment_for_token(&target_token)
        .map_err(|e| format!("failed to create environment for child process: {e}"))?;

    let current_exe = std::env::current_exe()
        .map_err(|e| format!("failed to resolve path to current executable: {e}"))?;

    let tcp = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("failed to open a local TCP connection: {e}"))?;
    let addr = tcp
        .local_addr()
        .map_err(|e| format!("failed to get info about local TCP connection: {e}"))?;

    command.set_argv({
        let mut args = app.get_args(addr.port());
        args.insert(0, OsString::from(current_exe));
        args
    });
    let proc = if should_elevate {
        command
            .shell_execute("runas")
            .map_err(|e| format!("failed to spawn elevated process: {e}"))?
    } else {
        command
            .spawn_with_token(&target_token)
            .map_err(|e| format!("failed to spawn child process: {e}"))?
    };
    let proc = into_process!(proc);

    let cancel = AtomicBool::new(false);
    let shared_stream = OnceLock::<TcpStream>::new();
    std::thread::scope(|s| -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        let handle = s.spawn(|| {
            let tx = tx;
            let (stream, _addr) = match tcp.accept() {
                Ok(v) => v,
                Err(e) => {
                    let _ = tx.send(format!("failed to accept TCP connection: {e}"));
                    return
                }
            };

            let _ = shared_stream.set(stream);
            let mut stream = shared_stream.get().unwrap();

            if cancel.load(Ordering::Acquire) {
                return;
            }

            let confirm_msg = app.confirm_message();
            let mut data = vec![0; confirm_msg.len()];
            if let Err(e) = stream.read_exact(&mut data) {
                let _ = tx.send(format!("failed to read from TCP stream: {e}"));
                return;
            }

            if data.as_slice() != &*confirm_msg {
                let _ = tx.send(format!(
                    "Invalid data sent over TCP stream while waiting for restart confirmation message: {}",
                    String::from_utf8_lossy(&data)
                ));
                return;
            }

            app.exit();
        });
        let wait_result = loop {
            match proc.wait_for(Some(1000)) {
                // Child process started:
                Ok(_code) => break Ok(()),
                // Handle errors in the other thread:
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => match rx.try_recv() {
                    Ok(err) => {
                        handle.join().unwrap(); // Wait for thread to exit...
                        return Err(err); // Then return its error
                    }
                    // Other thread exited unexpectedly:
                    Err(TryRecvError::Disconnected) => {
                        return Err(
                            "Failed to wait for restarted process to confirm it had started"
                                .to_string(),
                        )
                    }
                    // Other thread hasn't made progress:
                    Err(TryRecvError::Empty) => {}
                },
                Err(e) => break Err(format!("failed to wait for child process to exit: {e}")),
            }
        };
        // Child process exited or we timed out (failed to change elevation!)

        // Notify other thread to exit:
        cancel.store(true, Ordering::Release);
        if let Some(stream) = shared_stream.get() {
            // Cancel work on the other thread.
            let _ = stream.shutdown(Shutdown::Both);
        } else {
            // Connect to the listener so that it unblocks:
            let _ = TcpStream::connect_timeout(&addr, Duration::from_millis(3000));
        }
        // Attempt to wait for other thread to exit:
        let _ = rx.recv_timeout(Duration::from_millis(10_000));

        wait_result
    })?;

    let code = proc
        .exit_code()
        .map_err(|e| format!("failed to get exit code for child process: {e}"))?;

    Err(format!("Failed to spawn child process (exit code: {code})"))
}

pub struct AdminRestart;
impl AdminRestart {
    const RESTARTED_ARG: &'static str = "restarted";
    const RESTART_TCP_MSG: &'static str = "restarted-backup-manager";

    pub fn handle_startup(&self) {
        if std::env::args().nth(1).as_deref() == Some(Self::RESTARTED_ARG) {
            use std::io::Write;

            tracing::info!(
                args = ?std::env::args().skip(2).collect::<Vec<_>>(),
                "Program was restarted"
            );

            let port: u16 = std::env::args()
                .nth(2)
                .expect("2nd arg should be a port number")
                .parse()
                .expect("2nd arg should be a 16bit number");

            tracing::debug!(
                "Notifying parent process at port {port} that we have successfully started"
            );

            let mut stream = std::net::TcpStream::connect_timeout(
                &([127, 0, 0, 1], port).into(),
                std::time::Duration::from_millis(1500),
            )
            .expect("failed to connect to parent process");

            tracing::trace!("Writing message to parent process to confirm that we have started");

            stream
                .write_all(Self::RESTART_TCP_MSG.as_bytes())
                .expect("failed to write data to parent process");

            drop(stream);
            // Wait for parent process to exit (only one instance of the app
            // should be running):
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
}
impl SetElevationHandler for AdminRestart {
    fn get_args(&mut self, port: u16) -> Vec<OsString> {
        vec![
            OsString::from(Self::RESTARTED_ARG),
            OsString::from(port.to_string()),
        ]
    }

    fn exit(&mut self) -> ! {
        std::process::exit(0);
    }

    fn confirm_message(&mut self) -> Cow<'_, [u8]> {
        Cow::Borrowed(Self::RESTART_TCP_MSG.as_bytes())
    }
}
