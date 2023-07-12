use std::{fmt, num::ParseIntError, time::Duration};

use serde::{de, Deserializer};
use thiserror::Error;

pub fn parse(s: &str) -> Result<Duration, ParseError> {
    let s = s.trim();
    if s.contains(char::is_whitespace) {
        Err(ParseError::ContainsWhitespace)
    } else if let Some(ms) = s.strip_suffix("ms") {
        Ok(Duration::from_millis(ms.parse()?))
    } else if let Some(us) = s.strip_suffix("us") {
        Ok(Duration::from_micros(us.parse()?))
    } else if let Some(s) = s.strip_suffix('s') {
        Ok(Duration::from_secs(s.parse()?))
    } else {
        Err(ParseError::UnknownUnit)
    }
}

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("the duration cannot contain whitespace")]
    ContainsWhitespace,
    #[error("the duration has an unknown unit, must be: s, ms, or us")]
    UnknownUnit,
    #[error("the duration could not be parsed as an integer")]
    InvalidInt(#[from] ParseIntError),
}

pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
    deserializer.deserialize_str(Visitor)
}

#[derive(Debug)]
struct Visitor;

impl<'de> de::Visitor<'de> for Visitor {
    type Value = Duration;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a string with 's', 'ms', or 'us' suffix")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        parse(v).map_err(E::custom)
    }
}

pub mod option {
    use std::{fmt, time::Duration};

    use serde::{de, Deserializer};

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Duration>, D::Error> {
        deserializer.deserialize_option(Visitor)
    }

    #[derive(Debug)]
    struct Visitor;

    impl<'de> de::Visitor<'de> for Visitor {
        type Value = Option<Duration>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("option")
        }

        fn visit_some<D: Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<Self::Value, D::Error> {
            super::deserialize(deserializer).map(Some)
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
    }
}
