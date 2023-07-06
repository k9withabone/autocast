use std::{
    ffi::OsStr,
    io::{self, BufRead, BufReader, Read, Write},
    ops::{Deref, DerefMut},
    process::Command,
    time::{Duration, Instant},
};

use color_eyre::eyre::{self, Context};
#[cfg(unix)]
use expectrl::process::unix::UnixProcess;
#[cfg(windows)]
use expectrl::process::windows::WinProcess;
use expectrl::{
    process::{NonBlocking, Process},
    session::{OsProcess, OsProcessStream},
};
use os_str_bytes::OsStrBytes;

use crate::asciicast::Event;

pub(super) fn bash<I, K, V>(
    timeout: Duration,
    environment: I,
    width: u16,
    height: u16,
) -> color_eyre::Result<ShellSession>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    const PROMPT: &str = "AUTOCAST_PROMPT";
    const PROMPT_COMMAND: &str =
        "PS1=AUTOCAST_PROMPT; unset PROMPT_COMMAND; bind 'set enable-bracketed-paste off'";

    let mut command = Command::new("bash");
    command
        .envs(environment)
        .env("PS1", PROMPT)
        .env("PROMPT_COMMAND", PROMPT_COMMAND);

    ShellSession::spawn(
        command,
        width,
        height,
        String::from(PROMPT),
        Some(String::from("exit")),
        timeout,
    )
}

pub(super) fn python<I, K, V>(
    timeout: Duration,
    environment: I,
    width: u16,
    height: u16,
) -> color_eyre::Result<ShellSession>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    let mut command = Command::new("python");
    command.envs(environment);

    ShellSession::spawn(
        command,
        width,
        height,
        String::from(">>> "),
        Some(String::from("exit()")),
        timeout,
    )
}

pub struct ShellSession<P = OsProcess, S = OsProcessStream> {
    prompt: String,
    quit_command: Option<String>,
    timeout: Duration,
    process: P,
    stream: Stream<S>,
    last_event: Instant,
}

impl<P, S> ShellSession<P, S> {
    fn new_event(&mut self, data: String) -> Event {
        let now = Instant::now();
        let event = Event::output(now - self.last_event, data);
        self.last_event = now;
        event
    }

    /// Reset the time when the last event occurred to now.
    pub fn reset(&mut self) {
        self.last_event = Instant::now();
    }
}

impl<P, S: Read> ShellSession<P, S> {
    fn new(
        prompt: String,
        quit_command: Option<String>,
        timeout: Duration,
        process: P,
        stream: S,
    ) -> Self {
        let now = Instant::now();
        Self {
            prompt,
            quit_command,
            timeout,
            process,
            stream: Stream::new(stream),
            last_event: now,
        }
    }

    /// Reads the shell's output, adding it to the event buffer.
    /// Returns whether the prompt was detected.
    pub fn read(&mut self) -> io::Result<(Option<Event>, bool)> {
        let data = self.stream.read_to_string()?;

        if data.is_empty() {
            Ok((None, false))
        } else if let Some((data, _)) = data.rsplit_once(&self.prompt) {
            if data.is_empty() {
                Ok((None, true))
            } else {
                Ok((Some(self.new_event(String::from(data))), true))
            }
        } else {
            Ok((Some(self.new_event(data)), false))
        }
    }

    /// Reads the shell's output, adding it to the event buffer,
    /// blocking until the prompt is detected, or the timeout is surpassed.
    ///
    /// # Errors
    ///
    /// Returns an error if the timeout is surpassed or there was an IO error
    /// while reading the shell output.
    pub fn read_until_prompt(&mut self) -> color_eyre::Result<(Vec<Event>, Duration)> {
        let start = Instant::now();
        let mut events = Vec::new();
        loop {
            let (event, prompt) = self.read().wrap_err("error reading shell output")?;
            events.extend(event);
            if prompt {
                return Ok((events, self.last_event.elapsed()));
            }
            if start.elapsed() > self.timeout {
                eyre::bail!("timeout elapsed");
            }
        }
    }
}

impl<P, S: Write> ShellSession<P, S> {
    /// Send the buffer to the shell's stdin.
    pub fn send(&mut self, buf: impl AsRef<[u8]>) -> io::Result<()> {
        self.stream.write_all(buf.as_ref())
    }

    /// Send the line to the shell's stdin, adding a new line to the end.
    pub fn send_line(&mut self, line: impl AsRef<[u8]>) -> io::Result<()> {
        #[cfg(not(windows))]
        const LINE_ENDING: &str = "\n";
        #[cfg(windows)]
        const LINE_ENDING: &str = "\r\n";

        self.send(line)?;
        let line_ending: &OsStr = LINE_ENDING.as_ref();
        self.send(line_ending.to_raw_bytes())
    }
}

impl<P: Process + WindowSize> ShellSession<P, P::Stream>
where
    P::Stream: Read,
{
    /// Spawn a new [`ShellSession`] from a [`Command`].
    /// Blocks until the shell's prompt is read.
    pub fn spawn(
        command: P::Command,
        width: u16,
        height: u16,
        prompt: String,
        quit_command: Option<String>,
        timeout: Duration,
    ) -> color_eyre::Result<Self> {
        let mut process = P::spawn_command(command).wrap_err("could not spawn process")?;
        let stream = process
            .open_stream()
            .wrap_err("could not open process stream")?;
        process
            .set_window_size(width, height)
            .wrap_err("could not set child terminal's size")?;
        let mut shell_session = Self::new(prompt, quit_command, timeout, process, stream);
        shell_session
            .read_until_prompt()
            .wrap_err("could not detect prompt")?;
        Ok(shell_session)
    }
}

impl<P: Process + Wait, S: Write> ShellSession<P, S> {
    /// Sends the quit command to the shell.
    /// Blocks until the shell process has exited.
    pub fn quit(&mut self) -> color_eyre::Result<()> {
        if let Some(quit_command) = &self.quit_command {
            let quit_command = quit_command.clone();
            self.send_line(quit_command)
                .wrap_err("error sending quit command to shell")?;
        }

        self.process
            .wait(self.timeout)
            .wrap_err("error waiting for shell to stop")?;

        Ok(())
    }
}

pub trait WindowSize: Process {
    fn set_window_size(&mut self, width: u16, height: u16) -> color_eyre::Result<()>;
}

#[cfg(unix)]
impl WindowSize for UnixProcess {
    fn set_window_size(&mut self, width: u16, height: u16) -> color_eyre::Result<()> {
        self.deref_mut()
            .set_window_size(width, height)
            .map_err(Into::into)
    }
}

#[cfg(windows)]
impl WindowSize for WinProcess {
    fn set_window_size(&mut self, width: u16, height: u16) -> color_eyre::Result<()> {
        let width = width.try_into().unwrap_or(i16::MAX);
        let height = height.try_into().unwrap_or(i16::MAX);
        self.deref_mut().resize(width, height).map_err(Into::into)
    }
}

pub trait Wait: Process {
    /// Waits for process to finish.
    fn wait(&self, timeout: Duration) -> color_eyre::Result<()>;
}

#[cfg(unix)]
impl Wait for UnixProcess {
    fn wait(&self, _: Duration) -> color_eyre::Result<()> {
        self.deref().wait().map(|_| ()).map_err(Into::into)
    }
}

#[cfg(windows)]
impl Wait for WinProcess {
    fn wait(&self, timeout: Duration) -> color_eyre::Result<()> {
        let timeout = timeout.as_millis().try_into().unwrap_or(u32::MAX);
        self.deref()
            .wait(Some(timeout))
            .map(|_| ())
            .map_err(Into::into)
    }
}

#[derive(Debug)]
struct Stream<S> {
    inner: BufReader<S>,
    buffer: Vec<u8>,
}

impl<S: Read> Read for Stream<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<S: Read> BufRead for Stream<S> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt);
    }
}

impl<S: Write> Write for Stream<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.get_mut().write(buf)
    }

    fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        self.inner.get_mut().write_vectored(bufs)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.get_mut().flush()
    }
}

impl<S: NonBlocking> NonBlocking for Stream<S> {
    fn set_non_blocking(&mut self) -> io::Result<()> {
        self.inner.get_mut().set_non_blocking()
    }

    fn set_blocking(&mut self) -> io::Result<()> {
        self.inner.get_mut().set_blocking()
    }
}

impl<S: Read> Stream<S> {
    fn new(inner: S) -> Self {
        Self {
            inner: BufReader::new(inner),
            buffer: vec![0; 2048],
        }
    }

    fn read_to_string(&mut self) -> io::Result<String> {
        let bytes_read = self.inner.read(&mut self.buffer)?;
        let string = OsStr::assert_from_raw_bytes(&self.buffer[..bytes_read]);
        Ok(string.to_string_lossy().into())
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;

    const TEST: &str = "test";

    #[cfg(target_os = "linux")]
    fn bash() -> color_eyre::Result<ShellSession> {
        super::bash(
            Duration::from_millis(500),
            std::iter::empty::<(&str, &str)>(),
            80,
            24,
        )
    }

    fn empty_stream() -> ShellSession<(), io::Empty> {
        ShellSession::new(String::new(), None, Duration::ZERO, (), io::empty())
    }

    fn test_bytes() -> Cow<'static, [u8]> {
        OsStr::new(TEST).to_raw_bytes()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bash_output_and_timing() -> color_eyre::Result<()> {
        let mut shell_session = bash()?;
        shell_session.reset();
        shell_session.send_line("echo test && sleep 0.01")?;
        let (output, last_prompt) = shell_session.read_until_prompt()?;
        shell_session.quit()?;
        let output: Vec<_> = output
            .into_iter()
            .map(|event| {
                assert!(event.time > Duration::ZERO);
                event.data
            })
            .collect();
        assert_eq!(output, ["test\r\n"]);
        assert!(last_prompt > Duration::from_millis(10));
        Ok(())
    }

    #[test]
    fn new_event() {
        let mut shell_session = empty_stream();
        let start = shell_session.last_event;
        let event = shell_session.new_event(String::from(TEST));

        assert!(shell_session.last_event > start);
        assert!(!event.time.is_zero());
        assert_eq!(event.data, TEST);
    }

    #[test]
    fn read_empty() {
        let mut shell_session = empty_stream();
        assert_eq!(shell_session.read().unwrap(), (None, false));
    }

    #[test]
    fn read_no_prompt() {
        let bytes = test_bytes();
        let mut shell_session = ShellSession::new(
            String::from("PROMPT"),
            None,
            Duration::ZERO,
            (),
            bytes.as_ref(),
        );
        let (event, prompt) = shell_session.read().unwrap();
        let event = event.unwrap();
        assert!(!event.time.is_zero());
        assert_eq!(event.data, TEST);
        assert!(!prompt);
    }

    #[test]
    fn read_prompt_only() {
        let bytes = test_bytes();
        let mut shell_session =
            ShellSession::new(String::from(TEST), None, Duration::ZERO, (), bytes.as_ref());
        assert_eq!(shell_session.read().unwrap(), (None, true));
    }

    #[test]
    fn read_output_and_prompt() {
        let output = "output";
        let bytes = [OsStr::new(output).to_raw_bytes(), test_bytes()].concat();
        let mut shell_session = ShellSession::new(
            String::from(TEST),
            None,
            Duration::ZERO,
            (),
            bytes.as_slice(),
        );
        let (event, prompt) = shell_session.read().unwrap();
        let event = event.unwrap();
        assert!(!event.time.is_zero());
        assert_eq!(event.data, output);
        assert!(prompt);
    }

    #[test]
    fn stream_read_to_string() {
        let bytes = test_bytes();
        let mut stream = Stream::new(bytes.as_ref());
        assert_eq!(stream.read_to_string().unwrap(), TEST);
    }
}
