use std::{
    ffi::OsStr,
    io::{self, Read, Write},
    mem,
    process::Command,
    time::Instant,
};

use color_eyre::eyre::Context;
use expectrl::{
    process::{NonBlocking, Process},
    session::{OsProcess, OsProcessStream},
};

use crate::asciicast::Event;

use super::Duration;

pub(super) type ReplSession = expectrl::repl::ReplSession<OsProcess, EventStream<OsProcessStream>>;

pub(super) fn bash<I, K, V>(
    timeout: Option<Duration>,
    environment: I,
    width: u16,
    height: u16,
) -> color_eyre::Result<ReplSession>
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

    custom(
        command,
        timeout,
        width,
        height,
        String::from(PROMPT),
        Some(String::from("exit")),
        false,
    )
}

pub(super) fn python<I, K, V>(
    timeout: Option<Duration>,
    environment: I,
    width: u16,
    height: u16,
) -> color_eyre::Result<ReplSession>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    let mut command = Command::new("python");
    command.envs(environment);

    custom(
        command,
        timeout,
        width,
        height,
        String::from(">>> "),
        Some(String::from("exit()")),
        false,
    )
}

pub(super) fn custom(
    command: Command,
    timeout: Option<Duration>,
    width: u16,
    height: u16,
    prompt: String,
    quit_command: Option<String>,
    echo: bool,
) -> color_eyre::Result<ReplSession> {
    let mut session = session(command, prompt.clone()).wrap_err("could not start pty session")?;
    session.set_expect_timeout(timeout.map(Into::into));
    session
        .get_process_mut()
        .set_window_size(width, height)
        .wrap_err("could not set child terminal's size")?;

    let mut repl_session = ReplSession::new(session, prompt, quit_command, echo);
    repl_session
        .expect_prompt()
        .wrap_err("could not detect prompt")?;
    repl_session.get_stream_mut().reset();

    Ok(repl_session)
}

type Session = expectrl::Session<OsProcess, EventStream<OsProcessStream>>;

/// Similar to `Session::spawn()`, but wraps stream in `EventStream`
fn session(command: Command, prompt: String) -> Result<Session, expectrl::Error> {
    let mut process = OsProcess::spawn_command(command)?;
    let stream = process.open_stream()?;
    let stream = EventStream::new(stream, prompt);
    Ok(Session::new(process, stream)?)
}

pub(super) struct EventStream<S> {
    stream: S,
    prompt: String,
    events: Vec<Event>,
    last: Instant,
    last_prompt: Instant,
}

impl<S: Write> Write for EventStream<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }

    fn write_vectored(&mut self, bufs: &[io::IoSlice]) -> io::Result<usize> {
        self.stream.write_vectored(bufs)
    }
}

impl<S: Read> Read for EventStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let bytes_read = self.stream.read(buf)?;
        let data = std::str::from_utf8(&buf[..bytes_read])
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if data == self.prompt {
            self.last_prompt = Instant::now();
        } else {
            self.add_event(String::from(data));
        }
        Ok(bytes_read)
    }
}

impl<S: NonBlocking> NonBlocking for EventStream<S> {
    fn set_non_blocking(&mut self) -> io::Result<()> {
        self.stream.set_non_blocking()
    }

    fn set_blocking(&mut self) -> io::Result<()> {
        self.stream.set_blocking()
    }
}

impl<S> EventStream<S> {
    pub fn new(stream: S, prompt: String) -> Self {
        let now = Instant::now();
        Self {
            stream,
            prompt,
            events: Vec::new(),
            last: now,
            last_prompt: now,
        }
    }

    /// Take the events from internal buffer.
    /// Events have times that are difference between the last event (or the reset).
    /// Associated duration is the last time (since the previous event) the prompt was seen
    pub fn take_events(&mut self) -> (Vec<Event>, std::time::Duration) {
        let last_prompt = self.last_prompt.saturating_duration_since(self.last);
        (mem::take(&mut self.events), last_prompt)
    }

    pub fn reset(&mut self) {
        self.events.clear();
        self.last = Instant::now();
    }

    fn add_event(&mut self, data: String) {
        let now = Instant::now();
        self.events.push(Event::output(now - self.last, data));
        self.last = now;
    }
}
