use std::fmt;

use itertools::Itertools;
use serde::{
    de::{self, EnumAccess, VariantAccess},
    Deserialize,
};

use crate::config::Key;

use super::{control_from_variant, duration, parse_control};

#[derive(Deserialize)]
#[serde(variant_identifier)]
enum Variant {
    Char,
    Str,
    Control,
    Wait,
}

pub(in crate::config) struct Visitor;

impl<'de> de::Visitor<'de> for Visitor {
    type Value = Key;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("char, control char, duration string, or enum")
    }

    fn visit_char<E: de::Error>(self, v: char) -> Result<Self::Value, E> {
        Ok(Key::Char(v))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        if let Some(control) = v.strip_prefix('^') {
            Ok(Key::Control(parse_control(control)?))
        } else if let Some(str) = v.strip_prefix("!Str ") {
            Ok(Key::String(str.to_string()))
        } else if let Ok(char) = v.chars().exactly_one() {
            Ok(Key::Char(char))
        } else {
            let duration = duration::parse(v).map_err(E::custom)?;
            Ok(Key::Wait(duration))
        }
    }

    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<Self::Value, A::Error> {
        let (tag, variant) = data.variant()?;
        match tag {
            Variant::Char => Ok(Key::Char(variant.newtype_variant()?)),
            Variant::Str => Ok(Key::String(variant.newtype_variant()?)),
            Variant::Control => Ok(Key::Control(control_from_variant(variant)?)),
            Variant::Wait => {
                let wait: &str = variant.newtype_variant()?;
                let duration = duration::parse(wait).map_err(de::Error::custom)?;
                Ok(Key::Wait(duration))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use expectrl::ControlCode;

    use super::*;

    #[test]
    fn visit_char() -> serde_yaml::Result<()> {
        assert_eq!(serde_yaml::from_str::<Key>("t")?, Key::Char('t'));
        Ok(())
    }

    #[test]
    fn visit_str() -> serde_yaml::Result<()> {
        let keys: Vec<Key> = serde_yaml::from_str(
            "
            - t
            - ^m
            - 1s
            ",
        )?;
        assert_eq!(keys[0], Key::Char('t'));
        assert_eq!(keys[1], Key::Control(ControlCode::CarriageReturn));
        assert_eq!(keys[2], Key::Wait(Duration::from_secs(1)));
        Ok(())
    }

    #[test]
    fn visit_enum() -> serde_yaml::Result<()> {
        let keys: Vec<Key> = serde_yaml::from_str(
            "
            - !Char t
            - !Control m
            - !Wait 1s
            ",
        )?;
        assert_eq!(keys[0], Key::Char('t'));
        assert_eq!(keys[1], Key::Control(ControlCode::CarriageReturn));
        assert_eq!(keys[2], Key::Wait(Duration::from_secs(1)));
        Ok(())
    }
}
