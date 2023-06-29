use std::{
    collections::HashMap,
    io::{self, Write},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{
    ser::{Error, SerializeSeq, SerializeStruct},
    Serialize, Serializer,
};

#[derive(Debug, Clone)]
pub struct File {
    pub header: Header,
    pub events: Vec<Event>,
}

impl File {
    pub fn write(&self, mut writer: impl Write) -> io::Result<()> {
        self.header.serialize(&mut serializer(&mut writer))?;
        writeln!(writer)?;

        for event in &self.events {
            event.serialize(&mut serializer(&mut writer))?;
            writeln!(writer)?;
        }

        writer.flush()
    }
}

fn serializer<W: Write>(writer: W) -> serde_json::Serializer<W, Formatter> {
    serde_json::Serializer::with_formatter(writer, Formatter)
}

/// Formatter to match the output of `asciinema rec`
#[derive(Debug)]
struct Formatter;

impl serde_json::ser::Formatter for Formatter {
    fn write_f64<W>(&mut self, writer: &mut W, value: f64) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        write!(writer, "{value:.6}")
    }

    fn begin_array_value<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    fn begin_object_key<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    fn begin_object_value<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + io::Write,
    {
        writer.write_all(b": ")
    }
}

#[derive(Debug, Clone)]
pub struct Header {
    pub width: u16,
    pub height: u16,
    pub timestamp: Option<SystemTime>,
    pub duration: Option<Duration>,
    pub idle_time_limit: Option<f64>,
    pub command: Option<String>,
    pub title: Option<String>,
    pub env: HashMap<String, String>,
}

impl Serialize for Header {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut header = serializer.serialize_struct(
            "Header",
            // version, width, height
            3 + usize::from(self.timestamp.is_some())
                + usize::from(self.duration.is_some())
                + usize::from(self.idle_time_limit.is_some())
                + usize::from(self.command.is_some())
                + usize::from(self.title.is_some())
                + usize::from(!self.env.is_empty()),
        )?;

        header.serialize_field("version", &Self::VERSION)?;
        header.serialize_field("width", &self.width)?;
        header.serialize_field("height", &self.height)?;
        if let Some(timestamp) = &self.timestamp {
            if let Ok(timestamp) = timestamp.duration_since(UNIX_EPOCH) {
                header.serialize_field("timestamp", &timestamp.as_secs())?;
            } else {
                return Err(S::Error::custom("timestamp is before unix epoch"));
            }
        }
        if let Some(duration) = &self.duration {
            header.serialize_field("duration", &duration.as_secs_f64())?;
        }
        if let Some(idle_time_limit) = &self.idle_time_limit {
            header.serialize_field("idle_time_limit", idle_time_limit)?;
        }
        if let Some(command) = &self.command {
            header.serialize_field("command", command)?;
        }
        if let Some(title) = &self.title {
            header.serialize_field("title", title)?;
        }
        if !self.env.is_empty() {
            header.serialize_field("env", &self.env)?;
        }

        header.end()
    }
}

impl Header {
    const VERSION: u8 = 2;
}

#[derive(Debug, Clone)]
pub struct Event {
    pub time: Duration,
    pub event_type: EventType,
    pub data: String,
}

impl Serialize for Event {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut event = serializer.serialize_seq(Some(3))?;

        event.serialize_element(&self.time.as_secs_f64())?;
        event.serialize_element(&self.event_type)?;
        event.serialize_element(&self.data)?;

        event.end()
    }
}

impl Event {
    pub fn input(time: Duration, data: String) -> Self {
        Self {
            time,
            event_type: EventType::Input,
            data,
        }
    }

    pub fn output(time: Duration, data: String) -> Self {
        Self {
            time,
            event_type: EventType::Output,
            data,
        }
    }

    pub fn outputln(time: Duration) -> Self {
        Self {
            time,
            event_type: EventType::Output,
            data: String::from("\r\n"),
        }
    }

    pub fn marker(time: Duration, data: String) -> Self {
        Self {
            time,
            event_type: EventType::Marker,
            data,
        }
    }
}

#[derive(Debug, Clone)]
pub enum EventType {
    Input,
    Output,
    Marker,
}

impl Serialize for EventType {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let event_type = match self {
            Self::Input => "i",
            Self::Output => "o",
            Self::Marker => "m",
        };

        serializer.serialize_str(event_type)
    }
}
