use serde::{Deserialize, Serialize};
use std::fmt;

/// Defined rules, including suffix IDs. This is a key registry, not an assertion
/// that the corresponding product behavior has been implemented or tested.
pub const RULE_IDS: &[&str] = &[
    "M-01", "M-02", "M-03", "M-04", "M-05", "M-06", "M-07", "M-08", "M-09", "M-10", "M-11",
    "M-11a", "M-12", "M-12a", "M-13", "M-14", "M-15", "M-16", "M-17", "M-18", "M-19", "C-01",
    "C-02", "C-03", "C-04", "C-05", "C-06", "C-07", "C-08", "O-01", "O-02", "O-03", "O-04", "O-05",
    "O-06", "O-07", "O-08", "O-09", "O-10", "O-11", "O-12", "O-13", "P-01", "P-02", "R-01", "R-02",
    "R-03", "R-04", "R-05", "R-06", "R-07", "R-08", "U-01", "U-02", "U-03", "U-04", "U-05", "U-06",
    "U-07", "U-08",
];

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RuleId(String);

impl RuleId {
    pub fn parse(value: &str) -> Result<Self, String> {
        if RULE_IDS.contains(&value) {
            Ok(Self(value.to_owned()))
        } else {
            Err(format!("unknown rule key {value}"))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for RuleId {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<RuleId> for String {
    fn from(value: RuleId) -> Self {
        value.0
    }
}

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FixtureId(u8);

impl FixtureId {
    pub fn parse(value: &str) -> Result<Self, String> {
        let number = value.strip_prefix('F').and_then(|n| n.parse::<u8>().ok());
        match number {
            Some(n @ 1..=21) if value == format!("F{n}") => Ok(Self(n)),
            _ => Err("expected fixture ID F1 through F21 without zero padding".into()),
        }
    }

    pub fn all() -> impl Iterator<Item = Self> {
        (1..=21).map(Self)
    }
}

impl TryFrom<String> for FixtureId {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<FixtureId> for String {
    fn from(value: FixtureId) -> Self {
        value.to_string()
    }
}

impl fmt::Display for FixtureId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "F{}", self.0)
    }
}
