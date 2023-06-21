use std::{
    collections::HashMap,
    fmt::{self, Display},
    io::Read,
    iter,
    num::ParseIntError,
    str::FromStr,
    time::SystemTime,
};

use clap::Args;
use color_eyre::eyre;
use serde::Deserialize;
use thiserror::Error;

use crate::asciicast;

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

        let mut env: HashMap<_, _> = environment.into_iter().map(Into::into).collect();
        for env_var in environment_capture {
            env.entry(env_var)
                .or_insert_with_key(|key| std::env::var(key).unwrap_or_default());
        }
        env.insert(String::from("SHELL"), shell.program);

        todo!("run instructions to get events");

        let command = (!shell.is_default_bash()).then_some(shell.to_string());

        Ok(Self {
            header: asciicast::Header {
                width,
                height,
                timestamp: Some(SystemTime::now()),
                duration: None,
                idle_time_limit: None,
                command,
                title,
                env,
            },
            events: Vec::new(),
        })
    }
}

#[derive(Args, Deserialize, Debug, Default, Clone)]
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
    #[arg(short = 'd', long, visible_alias = "delay", default_value_t)]
    #[serde(default)]
    type_speed: Duration,

    /// The shell prompt to use in the asciicast output
    #[arg(long, default_value = DEFAULT_PROMPT)]
    #[serde(default = "default_prompt")]
    prompt: String,

    /// The shell secondary prompt to use in the asciicast output
    #[arg(long, default_value = DEFAULT_SECONDARY_PROMPT)]
    #[serde(default = "default_secondary_prompt")]
    secondary_prompt: String,
}

const DEFAULT_PROMPT: &str = "$ ";
fn default_prompt() -> String {
    String::from(DEFAULT_PROMPT)
}

const DEFAULT_SECONDARY_PROMPT: &str = "> ";
fn default_secondary_prompt() -> String {
    String::from(DEFAULT_SECONDARY_PROMPT)
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
        } = other;

        self.width.merge(width);
        self.height.merge(height);
        self.title.merge(title);
        self.shell.merge(shell);
        self.environment.merge(environment);
        self.environment_capture.merge(environment_capture);
        self.type_speed.merge(type_speed);
        if prompt != DEFAULT_PROMPT {
            self.prompt = prompt;
        }
        if secondary_prompt != DEFAULT_SECONDARY_PROMPT {
            self.secondary_prompt = secondary_prompt;
        }
    }
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
struct Shell {
    program: String,
    #[serde(default)]
    args: Vec<String>,
}

impl FromStr for Shell {
    type Err = ParseShellError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(ParseShellError::Empty);
        }
        let mut args = shlex::split(s).ok_or(ParseShellError::Split)?;
        let program = args.remove(0);
        Ok(Self { program, args })
    }
}

#[derive(Error, Debug)]
enum ParseShellError {
    #[error("shell cannot be empty")]
    Empty,
    #[error("could not successfully split shell command into args")]
    Split,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            program: String::from("bash"),
            args: Vec::new(),
        }
    }
}

impl Display for Shell {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let command = iter::once(&self.program)
            .chain(&self.args)
            .map(String::as_str);
        f.write_str(&shlex::join(command))
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
    fn is_default_bash(&self) -> bool {
        self.args.is_empty()
            && self
                .program
                .split('/')
                .last()
                .expect("split has at least one element")
                == "bash"
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

#[derive(Deserialize, Debug, Clone, Copy, PartialEq)]
enum Duration {
    Seconds(u64),
    Milliseconds(u64),
    Microseconds(u64),
}

impl Default for Duration {
    fn default() -> Self {
        Self::Milliseconds(100)
    }
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

impl Merge for Duration {
    fn merge(&mut self, other: Self) {
        if other != Self::default() {
            *self = other;
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
enum Instruction {
    Command {
        command: Command,
        #[serde(default)]
        timeout: Option<Duration>,
        // #[serde(default)]
        // do_not_wait_for_completion: bool,
    },
    Interactive {
        command: Command,
        keys: Vec<Key>,
    },
    Wait(Duration),
    Marker(String),
    Hide,
    Show,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_is_default_bash() {
        assert!(
            Shell::default().is_default_bash(),
            "Shell default is not bash"
        );
        assert!(
            Shell {
                program: String::from("/usr/bin/bash"),
                args: Vec::new()
            }
            .is_default_bash(),
            "`/usr/bin/bash` not detected as bash"
        );
        assert!(
            Shell {
                program: String::from("/bin/bash"),
                args: Vec::new()
            }
            .is_default_bash(),
            "`/bin/bash` not detected as bash"
        );
        assert!(
            !Shell {
                program: String::from("fish"),
                args: Vec::new()
            }
            .is_default_bash(),
            "`fish` detected as bash"
        );
        assert!(
            !Shell {
                program: String::from("bash"),
                args: vec![String::from("--rcfile"), String::from("file")]
            }
            .is_default_bash(),
            "`bash --rcfile file` detected as default bash"
        );
    }
}
