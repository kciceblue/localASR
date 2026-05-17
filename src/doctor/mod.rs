//! `localasr doctor` — non-interactive diagnostic probes.
//!
//! A `Check` is a named async operation that returns `Result<()>`. The runner
//! executes them in order, prints `PASS` or `FAIL: <msg>` per check, and
//! returns the count of failures (so the CLI can set the exit code).

pub mod probes;

use anyhow::Result;
use futures::future::BoxFuture;

pub struct Check {
    pub name: &'static str,
    pub run: Box<dyn FnOnce() -> BoxFuture<'static, Result<()>> + Send>,
}

impl Check {
    pub fn new<F, Fut>(name: &'static str, f: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        use futures::FutureExt;
        Check { name, run: Box::new(move || f().boxed()) }
    }
}

/// Run all checks sequentially. Prints results to stdout.
/// Returns the number of failures.
pub async fn run_all(checks: Vec<Check>) -> usize {
    let mut failures = 0;
    for check in checks {
        print!("  {} ... ", check.name);
        use std::io::Write;
        let _ = std::io::stdout().flush();
        match (check.run)().await {
            Ok(()) => println!("PASS"),
            Err(e) => {
                println!("FAIL: {e:#}");
                failures += 1;
            }
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn all_pass_returns_zero() {
        let checks = vec![
            Check::new("check_a", || async { Ok(()) }),
            Check::new("check_b", || async { Ok(()) }),
        ];
        assert_eq!(run_all(checks).await, 0);
    }

    #[tokio::test]
    async fn one_failure_counted() {
        let checks = vec![
            Check::new("ok", || async { Ok(()) }),
            Check::new("bad", || async { anyhow::bail!("nope") }),
            Check::new("ok2", || async { Ok(()) }),
        ];
        assert_eq!(run_all(checks).await, 1);
    }

    #[tokio::test]
    async fn all_fail_counted() {
        let checks = vec![
            Check::new("a", || async { anyhow::bail!("x") }),
            Check::new("b", || async { anyhow::bail!("y") }),
        ];
        assert_eq!(run_all(checks).await, 2);
    }
}
