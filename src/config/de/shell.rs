use std::fmt;

use serde::{
    de::{self, VariantAccess},
    Deserialize,
};

use crate::config::Shell;

#[derive(Deserialize)]
#[serde(variant_identifier)]
enum Variant {
    Bash,
    Python,
    Custom,
}

const CUSTOM_FIELDS: &[&str] = &["program", "args", "prompt", "line_split", "quit_command"];

#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "snake_case")]
enum CustomField {
    Program,
    Args,
    Prompt,
    LineSplit,
    QuitCommand,
}

/// Visitor for deserializing [`Shell`]
pub(in crate::config) struct Visitor;

impl<'de> de::Visitor<'de> for Visitor {
    type Value = Shell;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a string, map, or enum")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        match v {
            "bash" | "Bash" => Ok(Shell::Bash),
            "python" | "Python" => Ok(Shell::Python),
            _ => Err(E::invalid_value(
                de::Unexpected::Str(v),
                &"supported shell (e.g. bash or python) or a custom shell",
            )),
        }
    }

    fn visit_map<A: de::MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
        CustomVisitor.visit_map(map)
    }

    fn visit_enum<A: de::EnumAccess<'de>>(self, data: A) -> Result<Self::Value, A::Error> {
        let (tag, variant) = data.variant()?;
        match tag {
            Variant::Bash => variant.unit_variant().map(|_| Shell::Bash),
            Variant::Python => variant.unit_variant().map(|_| Shell::Python),
            Variant::Custom => variant.struct_variant(CUSTOM_FIELDS, CustomVisitor),
        }
    }
}

/// Visitor for deserializing [`Shell::Custom`]
struct CustomVisitor;

impl<'de> de::Visitor<'de> for CustomVisitor {
    type Value = Shell;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a map")
    }

    fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut program = None;
        let mut args = None;
        let mut prompt = None;
        let mut line_split = None;
        let mut quit_command = None;
        map_fields!(
            map,
            (CustomField::Program, program, "program"),
            (CustomField::Args, args, "args"),
            (CustomField::Prompt, prompt, "prompt"),
            (CustomField::LineSplit, line_split, "line_split"),
            (CustomField::QuitCommand, quit_command, "quit_command"),
        )?;
        let program = program.ok_or_else(|| de::Error::missing_field("program"))?;
        let args = args.unwrap_or_default();
        let prompt = prompt.ok_or_else(|| de::Error::missing_field("prompt"))?;
        let line_split = line_split.ok_or_else(|| de::Error::missing_field("line_split"))?;

        Ok(Shell::Custom {
            program,
            args,
            prompt,
            line_split,
            quit_command,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visit_str() -> serde_yaml::Result<()> {
        assert_eq!(serde_yaml::from_str::<Shell>("bash")?, Shell::Bash);
        assert_eq!(serde_yaml::from_str::<Shell>("python")?, Shell::Python);
        assert!(serde_yaml::from_str::<Shell>("custom").is_err());
        Ok(())
    }

    #[test]
    fn visit_map() -> serde_yaml::Result<()> {
        let shell: Shell = serde_yaml::from_str(
            "
            program: program
            args:
            - arg
            prompt: prompt
            line_split: split
            quit_command: quit
            ",
        )?;
        assert_eq!(
            shell,
            Shell::Custom {
                program: String::from("program"),
                args: vec![String::from("arg")],
                prompt: String::from("prompt"),
                line_split: String::from("split"),
                quit_command: Some(String::from("quit"))
            }
        );
        assert!(serde_yaml::from_str::<Shell>("program: program").is_err());
        Ok(())
    }

    #[test]
    fn visit_enum() -> serde_yaml::Result<()> {
        let shells: Vec<Shell> = serde_yaml::from_str(
            "
            - !Bash
            - !Python
            - !Custom
              program: program
              prompt: prompt
              line_split: split
            ",
        )?;
        assert_eq!(shells[0], Shell::Bash);
        assert_eq!(shells[1], Shell::Python);
        assert_eq!(
            shells[2],
            Shell::Custom {
                program: String::from("program"),
                args: Vec::new(),
                prompt: String::from("prompt"),
                line_split: String::from("split"),
                quit_command: None
            }
        );
        assert!(serde_yaml::from_str::<Shell>("!Custom").is_err());
        Ok(())
    }
}
