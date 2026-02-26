/// Apply text insertions to source code.
/// Insertions must be applied back-to-front to preserve offsets.
pub fn apply_insertions(source: &str, mut insertions: Vec<TextInsertion>) -> String {
    // Sort by position descending so earlier insertions don't shift later ones
    insertions.sort_by(|a, b| b.offset.cmp(&a.offset));

    let mut result = source.to_string();
    for insertion in insertions {
        let offset = insertion.offset as usize;
        if offset <= result.len() {
            result.insert_str(offset, &insertion.text);
        }
    }
    result
}

/// A text insertion at a specific byte offset.
#[derive(Debug, Clone)]
pub struct TextInsertion {
    pub offset: u32,
    pub text: String,
}
