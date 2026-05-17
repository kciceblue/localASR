//! Merge term-extraction results from multiple chunks into a single TermDb.
//!
//! Semantics:
//! - Dedup by `name` (case-sensitive — `kubectl` and `Kubectl` are distinct).
//! - On collision: union aliases (preserving first-seen order, dedup),
//!   keep the FIRST non-empty hint.
//! - `finalize()` sorts entries alphabetically by `name` for stable output.

use crate::terms::{Term, TermDb};
use std::collections::HashMap;

/// Merge `new` terms into `db`. Duplicates are folded per the module-level rules.
pub fn merge_into(db: &mut TermDb, new: Vec<Term>) {
    // Build an index of existing names → position in db.entries.
    let mut index: HashMap<String, usize> = HashMap::with_capacity(db.entries.len());
    for (i, t) in db.entries.iter().enumerate() {
        index.insert(t.name.clone(), i);
    }
    for incoming in new {
        if incoming.name.is_empty() {
            continue;
        }
        match index.get(&incoming.name).copied() {
            None => {
                index.insert(incoming.name.clone(), db.entries.len());
                db.entries.push(incoming);
            }
            Some(pos) => {
                let existing = &mut db.entries[pos];
                // Union aliases, preserving first-seen order.
                for a in incoming.aliases {
                    if !existing.aliases.iter().any(|x| x == &a) {
                        existing.aliases.push(a);
                    }
                }
                // Keep first non-empty hint.
                if existing.hint.is_empty() && !incoming.hint.is_empty() {
                    existing.hint = incoming.hint;
                }
            }
        }
    }
}

/// Sort the db's entries alphabetically by name. Call once after all
/// `merge_into()` calls have completed.
pub fn finalize(db: &mut TermDb) {
    db.entries.sort_by(|a, b| a.name.cmp(&b.name));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(name: &str, aliases: &[&str], hint: &str) -> Term {
        Term {
            name: name.into(),
            aliases: aliases.iter().map(|s| (*s).into()).collect(),
            hint: hint.into(),
        }
    }

    #[test]
    fn merge_into_empty_db_adds_all() {
        let mut db = TermDb { entries: vec![] };
        merge_into(&mut db, vec![t("a", &["x"], "h-a"), t("b", &[], "")]);
        assert_eq!(db.entries.len(), 2);
        assert_eq!(db.entries[0].name, "a");
        assert_eq!(db.entries[0].aliases, vec!["x"]);
    }

    #[test]
    fn merge_into_dedups_by_name() {
        let mut db = TermDb { entries: vec![t("k", &["a"], "first")] };
        merge_into(&mut db, vec![t("k", &["b"], "second")]);
        assert_eq!(db.entries.len(), 1);
        assert_eq!(db.entries[0].aliases, vec!["a", "b"]);
        // First non-empty hint wins.
        assert_eq!(db.entries[0].hint, "first");
    }

    #[test]
    fn merge_into_fills_empty_hint() {
        let mut db = TermDb { entries: vec![t("k", &[], "")] };
        merge_into(&mut db, vec![t("k", &[], "now-i-know")]);
        assert_eq!(db.entries[0].hint, "now-i-know");
    }

    #[test]
    fn merge_into_unions_aliases_no_dup() {
        let mut db = TermDb { entries: vec![t("k", &["a", "b"], "")] };
        merge_into(&mut db, vec![t("k", &["b", "c"], "")]);
        assert_eq!(db.entries[0].aliases, vec!["a", "b", "c"]);
    }

    #[test]
    fn merge_into_skips_empty_name() {
        let mut db = TermDb { entries: vec![] };
        merge_into(&mut db, vec![t("", &["x"], "h")]);
        assert!(db.entries.is_empty());
    }

    #[test]
    fn finalize_sorts_by_name() {
        let mut db = TermDb {
            entries: vec![t("kubectl", &[], ""), t("ansible", &[], ""), t("tokio", &[], "")],
        };
        finalize(&mut db);
        let names: Vec<&str> = db.entries.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["ansible", "kubectl", "tokio"]);
    }
}
