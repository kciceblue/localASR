pub mod schema;

use anyhow::{Context, Result};
use std::path::Path;
pub use schema::{Term, TermDb};

pub fn load(path: &Path) -> Result<TermDb> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading terms at {}", path.display()))?;
    let db: TermDb = toml::from_str(&raw)
        .with_context(|| format!("parsing terms at {}", path.display()))?;
    Ok(db)
}

/// Render the term DB as lines suitable for a system prompt.
/// Each line: `name — hint (sounds-like: "alias1", "alias2")`.
/// Empty aliases / hint are omitted gracefully.
pub fn render_prompt_block(db: &TermDb) -> String {
    let mut out = String::new();
    for t in &db.entries {
        out.push_str(&t.name);
        if !t.hint.is_empty() {
            out.push_str(" \u{2014} ");
            out.push_str(&t.hint);
        }
        if !t.aliases.is_empty() {
            out.push_str(" (sounds-like: ");
            for (i, a) in t.aliases.iter().enumerate() {
                if i > 0 { out.push_str(", "); }
                out.push('"');
                out.push_str(a);
                out.push('"');
            }
            out.push(')');
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[[terms]]
name = "kubectl"
aliases = ["cube control"]
hint = "Kubernetes CLI tool"

[[terms]]
name = "tokio"
aliases = []
hint = ""
"#;

    #[test]
    fn loads_valid_terms() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), SAMPLE).unwrap();
        let db = load(tmp.path()).unwrap();
        assert_eq!(db.entries.len(), 2);
        assert_eq!(db.entries[0].name, "kubectl");
        assert_eq!(db.entries[1].aliases.len(), 0);
    }

    #[test]
    fn rejects_missing_required_field() {
        let bad = r#"
[[terms]]
name = "only-name"
"#;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), bad).unwrap();
        assert!(load(tmp.path()).is_err());
    }

    #[test]
    fn rejects_unknown_field() {
        let bad = r#"
[[terms]]
name = "x"
aliases = []
hint = ""
extra = "nope"
"#;
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), bad).unwrap();
        assert!(load(tmp.path()).is_err());
    }

    #[test]
    fn prompt_block_includes_alias_and_hint() {
        let db = TermDb {
            entries: vec![Term {
                name: "kubectl".into(),
                aliases: vec!["cube control".into()],
                hint: "Kubernetes CLI tool".into(),
            }],
        };
        let s = render_prompt_block(&db);
        assert!(s.contains("kubectl"));
        assert!(s.contains("Kubernetes CLI tool"));
        assert!(s.contains("cube control"));
    }

    #[test]
    fn prompt_block_handles_empty_hint_and_aliases() {
        let db = TermDb {
            entries: vec![Term {
                name: "bare".into(),
                aliases: vec![],
                hint: "".into(),
            }],
        };
        let s = render_prompt_block(&db);
        assert_eq!(s.trim(), "bare");
    }
}
