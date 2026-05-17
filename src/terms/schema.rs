use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TermDb {
    #[serde(rename = "terms")]
    pub entries: Vec<Term>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Term {
    pub name: String,
    pub aliases: Vec<String>,
    pub hint: String,
}
