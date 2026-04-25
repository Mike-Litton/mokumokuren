//! Cheap binary detection. Mirrors git's heuristic: if the first 8 KiB of a
//! blob contains a NUL byte, treat it as binary.

const SNIFF_LIMIT: usize = 8 * 1024;

#[must_use]
pub fn is_binary(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(SNIFF_LIMIT)];
    head.contains(&0u8)
}

/// Count `\n` occurrences in the blob. Text that lacks a trailing newline
/// still counts as having content on the final line if any bytes are present.
#[must_use]
pub fn count_lines(bytes: &[u8]) -> u32 {
    if bytes.is_empty() {
        return 0;
    }
    let newlines = bytecount_nl(bytes);
    let trailing = u32::from(*bytes.last().unwrap() != b'\n');
    newlines.saturating_add(trailing)
}

#[allow(clippy::naive_bytecount)]
fn bytecount_nl(bytes: &[u8]) -> u32 {
    let count: usize = bytes.iter().filter(|&&b| b == b'\n').count();
    // Saturate deliberately: 4 GiB+ blobs are real (giant generated files); LOC for them is noise anyway.
    u32::try_from(count).unwrap_or(u32::MAX)
}
