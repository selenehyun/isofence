pub mod console;
pub mod json;

use crate::engine::EngineResult;

/// Reporter trait for outputting results.
pub trait Reporter {
    fn report(&self, result: &EngineResult);
}

/// Byte offset → (line_1indexed, col_1indexed, line_start_byte, line_end_byte).
///
/// Returns the line/column for the given byte offset, plus the byte range
/// of that line in the source (useful for extracting source text).
pub fn offset_to_location(source: &str, offset: u32) -> (usize, usize, usize, usize) {
    let offset = offset as usize;
    let bytes = source.as_bytes();
    let len = bytes.len();
    let offset = offset.min(len);

    // Find line start
    let mut line_start = offset;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }

    // Find line end
    let mut line_end = offset;
    while line_end < len && bytes[line_end] != b'\n' {
        line_end += 1;
    }

    // Count line number (1-indexed)
    let line = source[..offset].matches('\n').count() + 1;

    // Column (1-indexed, byte-based)
    let col = offset - line_start + 1;

    (line, col, line_start, line_end)
}

/// Extract the source text for a line given its byte range.
pub fn get_source_line(source: &str, line_start: usize, line_end: usize) -> &str {
    &source[line_start..line_end]
}
