use crate::reaper::common_types::ReaperBool;

use super::*;
use eyre::{bail, Result, WrapErr};
use nom::*;
use nom_supreme::*;
pub mod serde_impl {
    use super::*;

    use std::fmt;

    use serde::de::{self, Visitor};

    struct I32Visitor;

    impl<'de> Visitor<'de> for I32Visitor {
        type Value = i32;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("an integer between -2^31 and 2^31")
        }

        fn visit_i8<E>(self, value: i8) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(i32::from(value))
        }

        fn visit_i32<E>(self, value: i32) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value)
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            use std::i32;
            if value >= i64::from(i32::MIN) && value <= i64::from(i32::MAX) {
                Ok(value as i32)
            } else {
                Err(E::custom(format!("i32 out of range: {}", value)))
            }
        }

        // Similar for other methods:
        //   - visit_i16
        //   - visit_u8
        //   - visit_u16
        //   - visit_u32
        //   - visit_u64
    }
}

pub struct ReaperTimestamp(chrono::NaiveDateTime);
pub enum FieldValue {}

pub struct Field {
    name: String,
    values: Vec<FieldValue>,
}

pub struct Container {
    fields: Vec<Field>,
}

pub struct Notes {
    field_1: i32,
    field_2: i32,
}

pub struct Header {
    version: String,
    build: String,
    timestamp: ReaperTimestamp,
    notes: Notes,
    ripple: ReaperBool,
}

pub struct ReaperSaveFile {}

impl ReaperSaveFile {
    pub fn parse(input: &str) -> Result<Self> {
        bail!("not implemented");
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    pub const EXAMPLE_1: &str = include_str!("./barbarah-anne.rpp");

    #[test]
    fn test_parses() -> Result<()> {
        ReaperSaveFile::parse(EXAMPLE_1).map(|_| ())
    }
}
