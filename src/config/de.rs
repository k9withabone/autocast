//! Modules implementing custom deserialization

macro_rules! map_fields {
    ($map:ident, $(($field:pat, $opt:ident, $name:expr)),+ $(,)?) => {
        loop {
            match $map.next_key() {
                Ok(Some(key)) => match key {
                    $(
                        $field => {
                            if $opt.is_some() {
                                break Err(serde::de::Error::duplicate_field($name));
                            }
                            match $map.next_value() {
                                Ok(value) => $opt = Some(value),
                                Err(error) => break Err(error),
                            }
                        }
                    )+
                }
                Ok(None) => break Ok(()),
                Err(error) => break Err(error),
            }
        }
    };
}

pub mod command;
pub mod duration;
pub mod key;
pub mod shell;

use expectrl::ControlCode;
use itertools::Itertools;
use serde::de::{self, Unexpected, VariantAccess};

fn parse_control<E: de::Error>(control: &str) -> Result<ControlCode, E> {
    let char = control
        .chars()
        .exactly_one()
        .map_err(|_| E::invalid_value(Unexpected::Str(control), &"single control char"))?;
    char.try_into().map_err(|_| invalid_control(char))
}

fn control_from_variant<'de, V>(variant: V) -> Result<ControlCode, V::Error>
where
    V: VariantAccess<'de>,
    V::Error: de::Error,
{
    let char: char = variant.newtype_variant()?;
    char.try_into().map_err(|_| invalid_control(char))
}

fn invalid_control<E: de::Error>(char: char) -> E {
    E::invalid_value(Unexpected::Char(char), &"valid control char")
}
