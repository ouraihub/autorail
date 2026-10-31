/// Decodes as much of `pending` as possible, replacing invalid bytes with U+FFFD and keeping an
/// incomplete trailing sequence for the next read unless `end` is set.
pub(crate) fn decode_utf8(pending: &mut Vec<u8>, end: bool) -> String {
    let mut text = String::with_capacity(pending.len());
    let mut keep = 0;
    let mut chunks = pending.utf8_chunks().peekable();
    while let Some(chunk) = chunks.next() {
        text.push_str(chunk.valid());
        let invalid = chunk.invalid();
        if invalid.is_empty() {
            continue;
        }
        let incomplete_tail = !end
            && chunks.peek().is_none()
            && std::str::from_utf8(invalid).is_err_and(|error| error.error_len().is_none());
        if incomplete_tail {
            keep = invalid.len();
        } else {
            text.push('\u{fffd}');
        }
    }
    let used = pending.len() - keep;
    pending.drain(..used);
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_and_split_sequences_keep_progress() {
        let mut pending = b"a\xffb\xe4\xb8".to_vec();
        assert_eq!(decode_utf8(&mut pending, false), "a\u{fffd}b");
        assert_eq!(pending, [0xe4, 0xb8]);
        pending.extend_from_slice(b"\xad\x80c\xf0");
        assert_eq!(decode_utf8(&mut pending, false), "中\u{fffd}c");
        assert_eq!(decode_utf8(&mut pending, true), "\u{fffd}");
        assert!(pending.is_empty());
    }

    #[test]
    fn truncated_sequence_followed_by_ascii_is_replaced_once() {
        let mut pending = b"\xe4\xb8A".to_vec();
        assert_eq!(decode_utf8(&mut pending, false), "\u{fffd}A");
        assert!(pending.is_empty());
    }
}
