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

pub mod duration;
pub mod shell;
