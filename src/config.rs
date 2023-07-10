mod run;
mod spawn;

use std::{
    collections::HashMap,
    ffi::OsStr,
    fmt::{self, Display},
    io::Read,
    iter,
    num::ParseIntError,
    process,
    str::FromStr,
    time::SystemTime,
};

use clap::{Args, ValueEnum};
use color_eyre::eyre::{self, Context};
use itertools::Itertools;
use serde::Deserialize;
use thiserror::Error;

use crate::asciicast;

use self::spawn::ShellSession;

#[derive(Deserialize, Debug, Clone)]
pub struct Script {
    #[serde(default)]
    settings: Settings,
    instructions: Vec<Instruction>,
}

impl Script {
    pub fn try_from_yaml(reader: impl Read) -> serde_yaml::Result<Self> {
        serde_yaml::from_reader(reader)
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

        let (width, height) = terminal_size(width, height).ok_or(eyre::eyre!(
            "terminal width or height not provided and could not get terminal size"
        ))?;

        let line_split = shell.line_split().to_string();
        let program = shell.program();
        let shell_env = which::which(program).map_or_else(
            |_| String::from(program),
            |program| program.to_string_lossy().into_owned(),
        );

        let mut shell_session = shell
            .spawn(timeout.into(), environment.iter().map_into(), width, height)
            .wrap_err("could not start shell")?;

        let type_speed = type_speed.into();
        let events = run::instructions(
            &value.instructions,
            &prompt,
            &secondary_prompt,
            type_speed,
            &line_split,
            &mut shell_session,
        )
        .wrap_err("error running instructions")?;
        shell_session.quit().wrap_err("could not exit shell")?;

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

fn terminal_size(width: Option<u16>, height: Option<u16>) -> Option<(u16, u16)> {
    match (width, height) {
        (Some(width), Some(height)) => Some((width, height)),
        (None, _) | (_, None) => {
            terminal_size::terminal_size().map(|(terminal_width, terminal_height)| {
                (
                    width.unwrap_or(terminal_width.0),
                    height.unwrap_or(terminal_height.0),
                )
            })
        }
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

    fn program(&self) -> &str {
        match self {
            Self::Bash => "bash",
            Self::Python => "python",
            Self::Custom { program, .. } => program,
        }
    }

    fn spawn<I, K, V>(
        self,
        timeout: std::time::Duration,
        environment: I,
        width: u16,
        height: u16,
    ) -> color_eyre::Result<ShellSession>
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
            } => {
                let mut command = process::Command::new(program);
                command.args(args).envs(environment);
                ShellSession::spawn(command, width, height, prompt, quit_command, timeout)
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
    Wait(#[serde(with = "serde_yaml::with::singleton_map")] Duration),
    Marker(String),
    Clear,
}

#[derive(Deserialize, Debug, Clone)]
enum Command {
    SingleLine(String),
    MultiLine(Vec<String>),
    Control(char),
}

#[derive(Deserialize, Debug, Clone)]
enum Key {
    Char(char),
    Control(char),
    Wait(#[serde(with = "serde_yaml::with::singleton_map")] Duration),
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
