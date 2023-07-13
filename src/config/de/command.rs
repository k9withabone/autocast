use std::fmt;

use serde::{
    de::{self, EnumAccess, SeqAccess, Unexpected, VariantAccess},
    Deserialize,
};

use crate::config::Command;

use super::{control_from_variant, parse_control};

#[derive(Deserialize)]
#[serde(variant_identifier)]
enum Variant {
    SingleLine,
    MultiLine,
    Control,
}

pub(in crate::config) struct Visitor;

impl<'de> de::Visitor<'de> for Visitor {
    type Value = Command;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a string or enum")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        if let Some(control) = v.strip_prefix('^') {
            Ok(Command::Control(parse_control(control)?))
        } else if v.contains('\n') {
            let lines = v.lines().map(String::from).collect();
            Ok(Command::MultiLine(lines))
        } else {
            Ok(Command::SingleLine(String::from(v)))
        }
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
        if v.starts_with('^') || v.contains('\n') {
            self.visit_str(&v)
        } else {
            Ok(Command::SingleLine(v))
        }
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let size = seq.size_hint().unwrap_or_default();
        let mut strings = Vec::with_capacity(size);
        while let Some(string) = seq.next_element()? {
            strings.push(string);
        }
        Ok(Command::MultiLine(strings))
    }

    fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<Self::Value, A::Error> {
        let (tag, variant) = data.variant()?;
        match tag {
            Variant::SingleLine => {
                let command: String = variant.newtype_variant()?;
                if command.contains('\n') {
                    Err(de::Error::invalid_value(
                        Unexpected::Str(&command),
                        &"single line string",
                    ))
                } else {
                    Ok(Command::SingleLine(command))
                }
            }
            Variant::MultiLine => Ok(Command::MultiLine(variant.newtype_variant()?)),
            Variant::Control => Ok(Command::Control(control_from_variant(variant)?)),
        }
    }
}

#[cfg(test)]
mod tests {
    use expectrl::ControlCode;

    use super::*;

    #[test]
    fn visit_str() -> serde_yaml::Result<()> {
        let commands: Vec<Command> = serde_yaml::from_str(
            "
            - test
            - |
              test1
              test2
            - ^m
            ",
        )?;
        assert_eq!(commands[0], Command::SingleLine(String::from("test")));
        assert_eq!(
            commands[1],
            Command::MultiLine(vec![String::from("test1"), String::from("test2")])
        );
        assert_eq!(commands[2], Command::Control(ControlCode::CarriageReturn));
        assert!(serde_yaml::from_str::<Command>("^test")
            .unwrap_err()
            .to_string()
            .contains("single control char"));
        Ok(())
    }

    #[test]
    fn visit_seq() -> serde_yaml::Result<()> {
        assert_eq!(
            serde_yaml::from_str::<Command>(
                "
                - test1
                - test2
                - test3
                "
            )?,
            Command::MultiLine(vec![
                String::from("test1"),
                String::from("test2"),
                String::from("test3")
            ])
        );
        Ok(())
    }

    #[test]
    fn visit_enum() -> serde_yaml::Result<()> {
        assert_eq!(
            serde_yaml::from_str::<Command>("!SingleLine test")?,
            Command::SingleLine(String::from("test"))
        );
        assert!(
            serde_yaml::from_str::<Command>("!SingleLine 'test1\n\ntest2'")
                .unwrap_err()
                .to_string()
                .contains("single line string")
        );
        assert_eq!(
            serde_yaml::from_str::<Command>(
                "
                !MultiLine
                - test1
                - test2
                "
            )?,
            Command::MultiLine(vec![String::from("test1"), String::from("test2")])
        );
        assert_eq!(
            serde_yaml::from_str::<Command>("!Control m")?,
            Command::Control(ControlCode::CarriageReturn)
        );
        Ok(())
    }
}
