pub mod schema;
pub use schema::*;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Resolve the default config path per platform (XDG on Linux, AppData on Windows).
pub fn default_config_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "localasr")
        .context("could not resolve user config directory")?;
    Ok(dirs.config_dir().join("config.toml"))
}

/// Load + parse + interpolate ${ENV} placeholders.
pub fn load(path: &Path) -> Result<schema::Config> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading config at {}", path.display()))?;
    let interpolated = interpolate_env(&raw)?;
    let cfg: schema::Config = toml::from_str(&interpolated)
        .with_context(|| format!("parsing TOML at {}", path.display()))?;
    Ok(cfg)
}

/// Replaces ${NAME} with the value of env var NAME.
/// `\$` is an escape for a literal `$`.
/// Missing env vars are an error.
pub fn interpolate_env(s: &str) -> Result<String> {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'$' {
            out.push('$');
            i += 2;
            continue;
        }
        if c == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            let end = s[i + 2..]
                .find('}')
                .with_context(|| format!("unclosed ${{ at byte {i}"))?;
            let name = &s[i + 2..i + 2 + end];
            let val = std::env::var(name)
                .with_context(|| format!("env var ${{{name}}} not set"))?;
            out.push_str(&val);
            i = i + 2 + end + 1;
            continue;
        }
        let ch = s[i..].chars().next().expect("valid UTF-8 slice");
        out.push(ch);
        i += ch.len_utf8();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolates_single_var() {
        std::env::set_var("LOCALASR_TEST_KEY", "abc123");
        assert_eq!(interpolate_env("k=${LOCALASR_TEST_KEY}").unwrap(), "k=abc123");
    }

    #[test]
    fn interpolates_multiple_vars() {
        std::env::set_var("LOCALASR_TEST_A", "one");
        std::env::set_var("LOCALASR_TEST_B", "two");
        assert_eq!(
            interpolate_env("${LOCALASR_TEST_A}-${LOCALASR_TEST_B}").unwrap(),
            "one-two"
        );
    }

    #[test]
    fn escaped_dollar_passes_through() {
        assert_eq!(interpolate_env(r"price=\$5").unwrap(), "price=$5");
    }

    #[test]
    fn missing_var_errors() {
        let err = interpolate_env("${LOCALASR_DEFINITELY_NOT_SET_XYZ}").unwrap_err();
        assert!(err.to_string().contains("not set"));
    }

    #[test]
    fn literal_dollar_without_brace_passes() {
        assert_eq!(interpolate_env("$100").unwrap(), "$100");
    }

    #[test]
    fn loads_minimal_valid_config() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), MINIMAL_CONFIG).unwrap();
        let cfg = load(tmp.path()).unwrap();
        assert_eq!(cfg.hotkey.binding, "RightCtrl");
        assert_eq!(cfg.asr.model, "whisper-1");
    }

    #[test]
    fn rejects_unknown_field() {
        let bad = MINIMAL_CONFIG.to_string() + "\n[bogus]\nfoo = 1\n";
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), bad).unwrap();
        let err = load(tmp.path()).unwrap_err();
        let full = format!("{err:#}");
        assert!(full.to_lowercase().contains("unknown") || full.contains("bogus"));
    }

    #[test]
    fn non_ascii_passes_through_unchanged() {
        assert_eq!(interpolate_env("café").unwrap(), "café");
        assert_eq!(interpolate_env("\u{1F600}").unwrap(), "\u{1F600}");
    }

    #[test]
    fn unclosed_brace_errors() {
        let err = interpolate_env("${UNCLOSED").unwrap_err();
        assert!(format!("{err:#}").to_lowercase().contains("unclosed"));
    }

    const MINIMAL_CONFIG: &str = r#"
[hotkey]
binding = "RightCtrl"

[audio]
device = ""
sample_rate = 16000

[vad]
backend = "silero"
min_silence_ms = 400
max_chunk_ms = 5000

[asr]
base_url = "https://api.openai.com/v1"
api_key = "sk-test"
model = "whisper-1"
language = ""
timeout_ms = 15000

[editor]
base_url = "https://api.openai.com/v1"
api_key = "sk-test"
model = "gpt-4o-mini"
light_temperature = 0.0
heavy_temperature = 0.2
light_timeout_ms = 4000
heavy_timeout_ms = 15000

[editor.light]
enabled = true
context_chunks = 3
system_prompt = ""

[editor.heavy]
enabled = true
system_prompt = ""

[terms]
enabled = false
path = "~/.config/localasr/terms.toml"

[injection]
mode = "clipboard_paste"
paste_shortcut = "Ctrl+V"
restore_delay_ms = 100
"#;
}
