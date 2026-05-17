//! Split a string into ≤max_bytes windows on line boundaries.
//!
//! Algorithm: accumulate lines into a buffer; whenever adding the next line
//! (including its trailing `\n`) would push the buffer past `max_bytes`,
//! emit the buffer as a chunk and start fresh. A single line that itself
//! exceeds `max_bytes` is emitted as its own chunk — we never split mid-line.

pub const DEFAULT_CHUNK_BYTES: usize = 8 * 1024;

pub fn chunk(text: &str, max_bytes: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut buf = String::new();
    for line in text.split_inclusive('\n') {
        // If the line itself is bigger than max_bytes:
        //   - flush whatever's accumulated (so order is preserved),
        //   - then emit the long line as its own chunk.
        if line.len() > max_bytes {
            if !buf.is_empty() {
                out.push(std::mem::take(&mut buf));
            }
            out.push(line.to_string());
            continue;
        }
        // If adding this line would exceed the cap, flush first.
        if !buf.is_empty() && buf.len() + line.len() > max_bytes {
            out.push(std::mem::take(&mut buf));
        }
        buf.push_str(line);
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_empty() {
        assert!(chunk("", 8 * 1024).is_empty());
    }

    #[test]
    fn small_input_is_one_chunk() {
        let out = chunk("hello\nworld\n", 8 * 1024);
        assert_eq!(out, vec!["hello\nworld\n"]);
    }

    #[test]
    fn splits_on_line_boundary() {
        // Each line is 10 bytes ("0123456789\n" is 11). Cap at 22 bytes per
        // chunk: we should get pairs of lines.
        let text: String = (0..6)
            .map(|i| format!("line-{i:03}\n")) // 10 bytes each
            .collect();
        let out = chunk(&text, 22);
        // 22 / 10 = 2.2 lines per chunk → 2 lines, since adding a 3rd would exceed.
        assert!(out.iter().all(|c| c.len() <= 22), "all chunks within cap");
        // Concatenation preserves the input.
        assert_eq!(out.concat(), text);
    }

    #[test]
    fn never_splits_mid_line() {
        let big_line = "a".repeat(20_000) + "\n";
        let text = format!("short\n{big_line}also-short\n");
        let out = chunk(&text, 8 * 1024);
        // The big line is emitted whole (chunk length > cap is allowed
        // specifically for over-sized single lines).
        assert!(out.iter().any(|c| c.len() > 8 * 1024));
        // No chunk is the big line concatenated with something else.
        assert!(out.iter().all(|c| c == &big_line || !c.contains(&big_line[..])));
        // Total preserved.
        assert_eq!(out.concat(), text);
    }

    #[test]
    fn trailing_text_without_newline_is_emitted() {
        let out = chunk("no-newline-at-end", 8 * 1024);
        assert_eq!(out, vec!["no-newline-at-end"]);
    }
}
