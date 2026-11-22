use serde::{Deserialize, Deserializer, Serialize, de};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "BTreeMap<String, ThinkingToggle>")]
pub struct ChatTemplateArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_thinking: Option<ThinkingToggle>,
}

impl TryFrom<BTreeMap<String, ThinkingToggle>> for ChatTemplateArgs {
    type Error = &'static str;

    fn try_from(mut fields: BTreeMap<String, ThinkingToggle>) -> Result<Self, Self::Error> {
        let enable_thinking = fields.remove("enable_thinking");
        if !fields.is_empty() {
            return Err("unknown chat template argument");
        }
        Ok(Self { enable_thinking })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ThinkingToggle {
    Fixed(bool),
    Variable(ThinkingVariable),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ThinkingVariable {
    #[serde(rename = "$var")]
    pub variable: ThinkingVariableName,
    #[serde(
        default,
        rename = "omitWhenOff",
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub omit_when_off: bool,
}

impl<'de> Deserialize<'de> for ThinkingVariable {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Object;
        impl<'de> de::Visitor<'de> for Object {
            type Value = ThinkingVariable;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a thinking variable object")
            }

            fn visit_map<M: de::MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let (mut variable, mut omit_when_off) = (None, None);
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "$var" if variable.is_none() => variable = Some(map.next_value()?),
                        "omitWhenOff" if omit_when_off.is_none() => {
                            omit_when_off = Some(map.next_value()?);
                        }
                        "$var" => return Err(de::Error::duplicate_field("$var")),
                        "omitWhenOff" => return Err(de::Error::duplicate_field("omitWhenOff")),
                        _ => return Err(de::Error::unknown_field(&key, &["$var", "omitWhenOff"])),
                    }
                }
                Ok(ThinkingVariable {
                    variable: variable.ok_or_else(|| de::Error::missing_field("$var"))?,
                    omit_when_off: omit_when_off.unwrap_or(false),
                })
            }
        }
        deserializer.deserialize_map(Object)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThinkingVariableName {
    #[serde(rename = "thinking.enabled")]
    Enabled,
}
