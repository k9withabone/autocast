mod spawn;

use std::{
    collections::HashMap,
    ffi::OsStr,
    fmt::{self, Display},
    io::Read,
    iter, mem,
    num::ParseIntError,
    process,
    str::FromStr,
    time::SystemTime,
};

use clap::{Args, ValueEnum};
use color_eyre::eyre::{self, Context};
use expectrl::ControlCode;
use itertools::Itertools;
use serde::Deserialize;
use thiserror::Error;

use crate::asciicast::{self, Event};

use self::spawn::ReplSession;

#[derive(Deserialize, Debug, Clone)]
pub struct Script {
    #[serde(default)]
    settings: Settings,
    instructions: Vec<Instruction>,
}

impl Script {
    pub fn try_from_ron(reader: impl Read) -> ron::error::SpannedResult<Self> {
        ron::de::from_reader(reader)
    }

    pub fn merge_settings(&mut self, other_settings: Settings) {
        self.settings.merge(other_settings);
    }
}

impl TryFrom<Script> for asciicast::File {
    type Error = color_eyre::Report;

    fn try_from(value: Script) -> Result<Self, Self::Error> {
        let Settings {
            width,
            height,
            title,
            shell,
            environment,
            environment_capture,
            type_speed,
            prompt,
            secondary_prompt,
            timeout,
        } = value.settings;

        let (width, height) = match (width, height) {
            (None, _) | (_, None) => terminal_size::terminal_size()
                .map(|(terminal_width, terminal_height)| {
                    (
                        width.unwrap_or(terminal_width.0),
                        height.unwrap_or(terminal_height.0),
                    )
                })
                .ok_or(eyre::eyre!(
                    "terminal width or height not provided and could not get terminal size"
                ))?,
            (Some(width), Some(height)) => (width, height),
        };

        let line_split = shell.line_split().to_string();
        let shell_env = match &shell {
            Shell::Bash => String::from("bash"),
            Shell::Python => String::from("python"),
            Shell::Custom { program, .. } => program.clone(),
        };

        let mut repl_session = shell
            .spawn(Some(timeout), environment.iter().map_into(), width, height)
            .wrap_err("could not start shell")?;

        let type_speed = type_speed.into();
        let events = value
            .instructions
            .iter()
            .scan(std::time::Duration::ZERO, |wait_time, instruction| {
                let events = instruction.run(
                    &prompt,
                    &secondary_prompt,
                    type_speed,
                    &line_split,
                    &mut repl_session,
                );
                let events = match events {
                    Ok(events) => events,
                    Err(error) => return Some(Err(error)),
                };
                if let Events::Wait(wait) = events {
                    *wait_time += wait.into();
                }
                let mut events = events.peekable();
                if *wait_time != std::time::Duration::ZERO {
                    if let Some(event) = events.peek_mut() {
                        event.time += mem::take(wait_time);
                    }
                }
                Some(Ok(events))
            })
            .process_results(|events| {
                iter::once(Event::output(std::time::Duration::ZERO, prompt.clone()))
                    .chain(events.flatten())
                    .chain(iter::once(Event::outputln(type_speed)))
                    .scan(std::time::Duration::ZERO, |time, mut event| {
                        event.time += *time;
                        *time = event.time;
                        Some(event)
                    })
                    .collect_vec()
            })
            .wrap_err("error running instruction")?;
        repl_session.exit().wrap_err("could not exit shell")?;

        let mut env: HashMap<_, _> = environment.into_iter().map_into().collect();
        for env_var in environment_capture {
            env.entry(env_var)
                .or_insert_with_key(|key| std::env::var(key).unwrap_or_default());
        }
        env.insert(String::from("SHELL"), shell_env);

        Ok(Self {
            header: asciicast::Header {
                width,
                height,
                timestamp: Some(SystemTime::now()),
                duration: None,
                idle_time_limit: None,
                command: None,
                title,
                env,
            },
            events,
        })
    }
}

#[derive(Args, Deserialize, Debug, Clone)]
pub struct Settings {
    /// Terminal width
    ///
    /// Default is the width of the current terminal
    #[arg(long)]
    #[serde(default)]
    width: Option<u16>,

    /// Terminal height
    ///
    /// Default is the height of the current terminal
    #[arg(long)]
    #[serde(default)]
    height: Option<u16>,

    /// Title of the asciicast
    #[arg(short, long)]
    #[serde(default)]
    title: Option<String>,

    /// Shell to use for running commands
    ///
    /// Will be listed in the asciicast's "env" header section as "SHELL"
    /// and in the "command" section
    ///
    /// To use a custom shell it must be specified in the input file
    #[arg(long, default_value_t)]
    #[serde(default)]
    shell: Shell,

    /// Environment variables to use in the shell process
    ///
    /// Will be listed in the asciicast's "env" header section
    ///
    /// If there are duplicates, the last value will take precedent
    #[arg(short, long, value_name = "NAME=VALUE")]
    #[serde(default)]
    environment: Vec<EnvVar>,

    /// Environment variables to capture
    ///
    /// Will be listed in the asciicast's "env" header section
    ///
    /// If there are duplicates with `--environment` values, those will take precedent
    #[arg(
        long,
        visible_alias = "env-cap",
        value_name = "ENV_VAR",
        default_value = "TERM"
    )]
    #[serde(default)]
    environment_capture: Vec<String>,

    /// Default time between key presses when writing commands
    ///
    /// Can be specified in seconds (s), milliseconds (ms), or microseconds (us)
    ///
    /// Use integers and the above abbreviations when specifying, i.e. "1s", "150ms", or "900us"
    #[arg(short = 'd', long, visible_alias = "delay", default_value_t = default_type_speed())]
    #[serde(default = "default_type_speed")]
    type_speed: Duration,

    /// The shell prompt to use in the asciicast output
    #[arg(long, default_value = DEFAULT_PROMPT)]
    #[serde(default = "default_prompt")]
    prompt: String,

    /// The shell secondary prompt to use in the asciicast output
    #[arg(long, default_value = DEFAULT_SECONDARY_PROMPT)]
    #[serde(default = "default_secondary_prompt")]
    secondary_prompt: String,

    /// Maximum amount of time to let a shell command run before returning with an error
    ///
    /// Can be specified in seconds (s), milliseconds (ms), or microseconds (us)
    ///
    /// Use integers and the above abbreviations when specifying, i.e. "1s", "150ms", or "900us"
    #[arg(long, default_value_t = default_timeout())]
    #[serde(default = "default_timeout")]
    timeout: Duration,
}

const fn default_type_speed() -> Duration {
    Duration::Milliseconds(100)
}

const DEFAULT_PROMPT: &str = "$ ";
fn default_prompt() -> String {
    String::from(DEFAULT_PROMPT)
}

const DEFAULT_SECONDARY_PROMPT: &str = "> ";
fn default_secondary_prompt() -> String {
    String::from(DEFAULT_SECONDARY_PROMPT)
}

const fn default_timeout() -> Duration {
    Duration::Seconds(30)
}

impl Merge for Settings {
    /// Merges `other` into self, `other` takes priority, ignoring defaults in other
    ///
    /// # Example
    ///
    /// ```
    /// let mut settings = Settings::default();
    /// let other = Settings {
    ///     width: Some(100),
    ///     ..Default::default()
    /// }
    ///
    /// settings.merge(other);
    /// assert_eq!(settings.width, Some(100));
    /// ```
    fn merge(&mut self, other: Self) {
        let Self {
            width,
            height,
            title,
            shell,
            environment,
            environment_capture,
            type_speed,
            prompt,
            secondary_prompt,
            timeout,
        } = other;

        self.width.merge(width);
        self.height.merge(height);
        self.title.merge(title);
        self.shell.merge(shell);
        self.environment.merge(environment);
        self.environment_capture.merge(environment_capture);
        if type_speed != default_type_speed() {
            self.type_speed = type_speed;
        }
        if prompt != DEFAULT_PROMPT {
            self.prompt = prompt;
        }
        if secondary_prompt != DEFAULT_SECONDARY_PROMPT {
            self.secondary_prompt = secondary_prompt;
        }
        if timeout != default_timeout() {
            self.timeout = timeout;
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            width: None,
            height: None,
            title: None,
            shell: Shell::default(),
            environment: Vec::new(),
            environment_capture: Vec::new(),
            type_speed: default_type_speed(),
            prompt: default_prompt(),
            secondary_prompt: default_secondary_prompt(),
            timeout: default_timeout(),
        }
    }
}

#[derive(Deserialize, ValueEnum, Debug, Clone, PartialEq)]
enum Shell {
    Bash,
    Python,
    #[value(skip)]
    Custom {
        program: String,
        #[serde(default)]
        args: Vec<String>,
        prompt: String,
        line_split: String,
        #[serde(default)]
        quit_command: Option<String>,
        #[serde(default)]
        echo: bool,
    },
}

impl Default for Shell {
    fn default() -> Self {
        Self::Bash
    }
}

impl Display for Shell {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Bash => f.write_str("bash"),
            Self::Python => f.write_str("python"),
            Self::Custom { program, args, .. } => {
                let command = iter::once(program).chain(args).map(String::as_str);
                f.write_str(&shlex::join(command))
            }
        }
    }
}

impl Merge for Shell {
    fn merge(&mut self, other: Self) {
        if other != Self::default() {
            *self = other;
        }
    }
}

impl Shell {
    fn line_split(&self) -> &str {
        match self {
            Self::Bash | Self::Python => " \\",
            Self::Custom { line_split, .. } => line_split,
        }
    }

    fn spawn<I, K, V>(
        self,
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
        match self {
            Self::Bash => spawn::bash(timeout, environment, width, height),
            Self::Python => spawn::python(timeout, environment, width, height),
            Self::Custom {
                program,
                args,
                prompt,
                line_split: _,
                quit_command,
                echo,
            } => {
                let mut command = process::Command::new(program);
                command.args(args).envs(environment);
                spawn::custom(command, timeout, width, height, prompt, quit_command, echo)
            }
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
struct EnvVar {
    name: String,
    value: String,
}

impl From<&str> for EnvVar {
    fn from(value: &str) -> Self {
        if let Some((name, value)) = value.split_once('=') {
            Self {
                name: String::from(name),
                value: String::from(value),
            }
        } else {
            Self {
                name: String::from(value),
                value: String::new(),
            }
        }
    }
}

impl From<EnvVar> for (String, String) {
    fn from(value: EnvVar) -> Self {
        (value.name, value.value)
    }
}

impl<'a> From<&'a EnvVar> for (&'a String, &'a String) {
    fn from(value: &'a EnvVar) -> Self {
        (&value.name, &value.value)
    }
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq)]
enum Duration {
    Seconds(u64),
    Milliseconds(u64),
    Microseconds(u64),
}

impl Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Seconds(s) => write!(f, "{s}s"),
            Self::Milliseconds(ms) => write!(f, "{ms}ms"),
            Self::Microseconds(us) => write!(f, "{us}us"),
        }
    }
}

impl FromStr for Duration {
    type Err = ParseDurationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.contains(char::is_whitespace) {
            Err(ParseDurationError::ContainsWhitespace)
        } else if let Some(ms) = s.strip_suffix("ms") {
            Ok(Self::Milliseconds(ms.parse()?))
        } else if let Some(us) = s.strip_suffix("us") {
            Ok(Self::Microseconds(us.parse()?))
        } else if let Some(s) = s.strip_suffix('s') {
            Ok(Self::Seconds(s.parse()?))
        } else {
            Err(ParseDurationError::UnknownUnit)
        }
    }
}

#[derive(Error, Debug)]
enum ParseDurationError {
    #[error("the duration cannot contain whitespace")]
    ContainsWhitespace,
    #[error("the duration has an unknown unit, must be: s, ms, or us")]
    UnknownUnit,
    #[error("the duration could not be parsed as an integer")]
    InvalidInt(#[from] ParseIntError),
}

impl From<Duration> for std::time::Duration {
    fn from(value: Duration) -> Self {
        match value {
            Duration::Seconds(s) => Self::from_secs(s),
            Duration::Milliseconds(ms) => Self::from_millis(ms),
            Duration::Microseconds(us) => Self::from_micros(us),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
enum Instruction {
    Command {
        command: Command,
        #[serde(default)]
        hidden: bool,
        #[serde(default)]
        type_speed: Option<Duration>,
    },
    Interactive {
        command: Command,
        keys: Vec<Key>,
        #[serde(default)]
        type_speed: Option<Duration>,
    },
    Wait(Duration),
    Marker(String),
    Clear,
}

impl Instruction {
    fn run<'a>(
        &'a self,
        prompt: &'a str,
        secondary_prompt: &'a str,
        default_type_speed: std::time::Duration,
        line_split: &'a str,
        repl_session: &mut ReplSession,
    ) -> color_eyre::Result<Events<impl Iterator<Item = Event> + 'a, impl Iterator<Item = Event>>>
    {
        match self {
            Self::Command {
                command,
                hidden,
                type_speed,
            } => {
                command
                    .send(repl_session)
                    .wrap_err("could not send command to shell")?;
                repl_session
                    .expect_prompt()
                    .wrap_err("could not detect prompt")?;

                if *hidden {
                    return Ok(Events::None);
                }

                let (output, last_prompt) = repl_session.get_stream_mut().take_events();

                let type_speed = type_speed.map_or(default_type_speed, Into::into);
                let events = command
                    .events(type_speed, secondary_prompt, line_split)
                    .chain(output)
                    .chain(iter::once(Event::output(last_prompt, String::from(prompt))));

                Ok(Events::Command(events))
            }
            Self::Interactive {
                command,
                keys,
                type_speed,
            } => todo!(),
            Self::Wait(duration) => Ok(Events::Wait(*duration)),
            Self::Marker(data) => Ok(Events::once(Event::marker(
                std::time::Duration::ZERO,
                data.clone(),
            ))),
            Self::Clear => {
                let clear =
                    Event::output(default_type_speed, String::from("\r\x1b[H\x1b[2J\x1b[3J"));
                let prompt = Event::output(default_type_speed, String::from(prompt));
                Ok(Events::Clear([clear, prompt].into_iter()))
            }
        }
    }
}

#[derive(Debug, Clone)]
enum Events<Co, Cl> {
    Command(Co),
    Clear(Cl),
    Once(iter::Once<Event>),
    Wait(Duration),
    None,
}

impl<Co, Cl> Iterator for Events<Co, Cl>
where
    Co: Iterator<Item = Event>,
    Cl: Iterator<Item = Event>,
{
    type Item = Event;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Command(iter) => iter.next(),
            Self::Clear(iter) => iter.next(),
            Self::Once(iter) => iter.next(),
            Self::Wait(_) | Self::None => None,
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Command(iter) => iter.size_hint(),
            Self::Clear(iter) => iter.size_hint(),
            Self::Once(iter) => iter.size_hint(),
            Self::Wait(_) | Self::None => (0, Some(0)),
        }
    }
}

impl<Co, Cl> Events<Co, Cl> {
    fn once(event: Event) -> Self {
        Self::Once(iter::once(event))
    }
}

#[derive(Deserialize, Debug, Clone)]
enum Command {
    SingleLine(String),
    MultiLine(Vec<String>),
    Control(char),
}

impl Command {
    fn send(&self, repl_session: &mut ReplSession) -> color_eyre::Result<()> {
        repl_session.get_stream_mut().reset();
        match self {
            Self::SingleLine(line) => repl_session.send_line(line)?,
            Self::MultiLine(lines) => repl_session.send_line(&lines.join(" "))?,
            Self::Control(char) => repl_session.send(
                ControlCode::try_from(*char).map_err(|_| eyre::eyre!("invalid control code"))?,
            )?,
        }
        Ok(())
    }

    fn events<'a>(
        &'a self,
        type_speed: std::time::Duration,
        secondary_prompt: &'a str,
        line_split: &'a str,
    ) -> impl Iterator<Item = Event> + 'a {
        match self {
            Self::SingleLine(line) => {
                CommandEvents::SingleLine(type_line(type_speed, line.chars()))
            }
            Self::MultiLine(lines) => {
                let num_lines = lines.len();
                let iter = lines.iter().enumerate().flat_map(move |(line_num, line)| {
                    let secondary_prompt = (line_num != 0)
                        .then(|| Event::output(type_speed, String::from(secondary_prompt)));

                    let line_split = (line_num + 1 < num_lines)
                        .then_some(line_split.chars())
                        .into_iter()
                        .flatten();

                    secondary_prompt
                        .into_iter()
                        .chain(type_line(type_speed, line.chars().chain(line_split)))
                });
                CommandEvents::MultiLine(iter)
            }
            Self::Control(char) => {
                CommandEvents::Control(type_line(type_speed, ['^', char.to_ascii_uppercase()]))
            }
        }
    }
}

#[derive(Debug, Clone)]
enum CommandEvents<S, M, C> {
    SingleLine(S),
    MultiLine(M),
    Control(C),
}

impl<S, M, C> Iterator for CommandEvents<S, M, C>
where
    S: Iterator<Item = Event>,
    M: Iterator<Item = Event>,
    C: Iterator<Item = Event>,
{
    type Item = Event;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::SingleLine(iter) => iter.next(),
            Self::MultiLine(iter) => iter.next(),
            Self::Control(iter) => iter.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::SingleLine(iter) => iter.size_hint(),
            Self::MultiLine(iter) => iter.size_hint(),
            Self::Control(iter) => iter.size_hint(),
        }
    }
}

fn type_line(
    type_speed: std::time::Duration,
    line: impl IntoIterator<Item = char>,
) -> impl Iterator<Item = Event> {
    line.into_iter()
        .map(move |char| Event::output(type_speed, String::from(char)))
        .chain(iter::once(Event::outputln(type_speed)))
}

#[derive(Deserialize, Debug, Clone)]
enum Key {
    Char(char),
    Control(char),
    Wait(Duration),
}

trait Merge {
    /// Merges `other` into self, `other` takes priority
    fn merge(&mut self, other: Self);
}

impl<T> Merge for Option<T> {
    fn merge(&mut self, other: Self) {
        if let Some(t) = other {
            self.replace(t);
        }
    }
}

impl<T> Merge for Vec<T> {
    fn merge(&mut self, other: Self) {
        self.extend(other);
    }
}
