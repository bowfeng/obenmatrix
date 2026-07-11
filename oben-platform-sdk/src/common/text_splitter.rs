/// Platform-specific text splitting utilities.
///
/// Following hermes-agent patterns for splitting messages that exceed
/// platform character limits. UTF-8 operations use `.chars()`, UTF-16 operations
/// use binary search to respect code unit boundaries.

/// Split a message that exceeds a platform's character limit.
///
/// Returns a Vec of segments, each at most `max_len` chars (NOT bytes).
/// Uses `.chars().take()` to be UTF-8 safe — never slices str at byte indices.
pub fn split_text(content: &str, max_len: usize) -> Vec<String> {
    if max_len == 0 {
        return vec![];
    }
    if content.is_empty() {
        return vec![];
    }

    let mut segments = Vec::new();
    let chars: Vec<char> = content.chars().collect();
    let total = chars.len();

    if total <= max_len {
        return vec![content.to_string()];
    }

    for chunk in chars.chunks(max_len) {
        segments.push(chunk.iter().collect());
    }

    segments
}

/// UTF-16 aware text splitting (for Telegram's 4096 UTF-16 code unit limit).
/// Uses binary search to respect code unit boundaries.
pub fn split_text_utf16(content: &str, max_len: usize) -> Vec<String> {
    if max_len == 0 {
        return vec![];
    }
    if content.is_empty() {
        return vec![];
    }

    let mut segments = Vec::new();
    let mut remaining = content;

    while !remaining.is_empty() {
        let utf16_len = remaining.encode_utf16().count();

        if utf16_len <= max_len {
            segments.push(remaining.to_string());
            break;
        }

        // Binary search for the largest prefix with <= max_len UTF-16 units
        let char_positions: Vec<(usize, char)> =
            remaining.char_indices().collect();

        let mut lo: usize = 0;
        let mut hi: usize = char_positions.len().saturating_sub(1);
        let mut best = 0usize;

        while lo <= hi {
            let mid = lo + (hi - lo) / 2;
            let cut_pos = char_positions[mid].0;
            let prefix = &remaining[..cut_pos];
            let prefix_utf16 = prefix.encode_utf16().count();

            if prefix_utf16 <= max_len {
                best = mid;
                lo = mid + 1;
            } else {
                hi = mid - 1;
            }
        }

        // If even the first character exceeds the limit, take one char (emoji)
        let cut_idx = if best == 0 {
            let first_char_len = remaining.chars().next().unwrap().len_utf8();
            remaining[..first_char_len].len()
        } else {
            // char_positions[best].0 is the byte offset of the character at index `best`
            // remaining[..cut_pos] gives us the prefix up to (but not including) that character
            // But we want to include the character at `best`, so we need to add its length
            // However, the binary search already confirmed that prefix up to char_positions[best].0
            // has UTF-16 len <= max_len, so we should use char_positions[best].0 directly
            char_positions[best].0
        };

        segments.push(
            remaining[..cut_idx]
                .chars()
                .map(|c| c.to_string())
                .collect::<String>(),
        );
        remaining = &remaining[cut_idx..];
    }

    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- split_text ---

    /// Given: An empty string
    /// When: split_text is called with any max_len
    /// Then: Returns an empty Vec
    #[test]
    fn test_split_text_empty() {
        assert!(split_text("", 100).is_empty());
    }

    /// Given: A string shorter than max_len
    /// When: split_text is called
    /// Then: Returns the original string as a single element
    #[test]
    fn test_split_text_under_limit() {
        let result = split_text("hello", 100);
        assert_eq!(result, vec!["hello"]);
    }

    /// Given: A string exactly at max_len
    /// When: split_text is called with max_len equal to char count
    /// Then: Returns the original string as a single element
    #[test]
    fn test_split_text_exact_limit() {
        let s = "hello";
        let result = split_text(s, 5);
        assert_eq!(result, vec!["hello"]);
    }

    /// Given: A simple ASCII string that exceeds max_len
    /// When: split_text is called with max_len = 5
    /// Then: Returns segments of at most 5 chars each
    #[test]
    fn test_split_text_ascii() {
        let result = split_text("Hello, World!", 5);
        assert_eq!(result, vec!["Hello", ", Wor", "ld!"]);
    }

    /// Given: max_len = 0
    /// When: split_text is called
    /// Then: Returns an empty Vec
    #[test]
    fn test_split_text_zero_max_len() {
        assert!(split_text("test", 0).is_empty());
    }

    /// Given: A Chinese/CJK string with multiple bytes per char
    /// When: split_text is called with max_len = 2
    /// Then: Splits at character boundaries, not byte boundaries
    #[test]
    fn test_split_text_cjk() {
        let cjk = "你好世界今天天气如何";
        let result = split_text(cjk, 2);
        assert_eq!(result, vec!["你好", "世界", "今天", "天气", "如何"]);
    }

    /// Given: A string with mixed ASCII and emoji
    /// When: split_text is called
    /// Then: Emoji counts as 1 character, not 4 bytes
    #[test]
    fn test_split_text_mixed() {
        let mixed = "Hi👋Hello"; // 7 chars total: H,i,👋,H,e,l,l,o
        let result = split_text(mixed, 3);
        assert_eq!(result, vec!["Hi👋", "Hel", "lo"]);
    }

    // --- split_text_utf16 ---

    /// Given: An empty string, max_len = 100
    /// When: split_text_utf16 is called
    /// Then: Returns an empty Vec
    #[test]
    fn test_split_utf16_empty() {
        assert!(split_text_utf16("", 100).is_empty());
    }

    /// Given: A string under the UTF-16 limit
    /// When: split_text_utf16 is called
    /// Then: Returns the original string as a single element
    #[test]
    fn test_split_utf16_under_limit() {
        let result = split_text_utf16("hello", 100);
        assert_eq!(result, vec!["hello"]);
    }

    /// Given: A string with ASCII chars each encoding as 1 UTF-16 unit
    /// When: split_text_utf16 is called with max_len = 5
    /// Then: Splits into segments of at most 5 code units
    #[test]
    fn test_split_utf16_ascii() {
        let result = split_text_utf16("Hello, World!", 5);
        // 5 UTF-16 code units per chunk: "Hello" (5), ", Wor" (5), "ld!" (3)
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "Hello");
        assert_eq!(result[1], ", Wor");
        assert_eq!(result[2], "ld!");
    }

    /// Given: A string with emoji (surrogate pairs, 2 UTF-16 units each)
    /// When: split_text_utf16 is called with a tight limit
    /// Then: Respects UTF-16 code unit boundaries
    #[test]
    fn test_split_utf16_emoji() {
        let with_emoji = "Hi👋Hello"; 
        let result = split_text_utf16(with_emoji, 3);
        // "Hi" (2 units) fits, then "👋H" (3 units) fits, then "ello" (4 units) needs split
        // Result: ["Hi", "👋H", "ell", "o"]
        assert_eq!(result.len(), 4);
        assert_eq!(result[0], "Hi");
        assert_eq!(result[1], "👋H");
        assert_eq!(result[2], "ell");
        let utf16_len = result[3].encode_utf16().count();
        assert!(utf16_len <= 3);
    }

    /// Given: A CJK string where chars each encode as 1 UTF-16 unit
    /// When: split_text_utf16 is called
    /// Then: Splits at character boundaries correctly
    #[test]
    fn test_split_utf16_cjk() {
        let cjk = "你好世界今天天气如何";
        let result = split_text_utf16(cjk, 2);
        // Each Chinese char = 1 UTF-16 unit, so split every 2 chars
        assert_eq!(result.len(), 5);
        assert_eq!(result[0], "你好");
        assert_eq!(result[1], "世界");
        assert_eq!(result[2], "今天");
        assert_eq!(result[3], "天气");
        assert_eq!(result[4], "如何");
    }

    /// Given: max_len = 0
    /// When: split_text_utf16 is called
    /// Then: Returns an empty Vec
    #[test]
    fn test_split_utf16_zero_max_len() {
        assert!(split_text_utf16("test", 0).is_empty());
    }
}
