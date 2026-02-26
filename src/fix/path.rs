use std::path::Path;

/// Compute relative path from a test file to a target module.
///
/// - Calculates relative path from test file's directory to target
/// - Removes .ts/.tsx/.js/.jsx extensions
/// - Converts to forward slashes
/// - Ensures "./" prefix for relative paths
pub fn compute_relative_path(test_file: &Path, target: &Path) -> String {
    let test_dir = test_file.parent().unwrap_or(Path::new("."));

    let rel = pathdiff::diff_paths(target, test_dir)
        .unwrap_or_else(|| target.to_path_buf());

    let mut path_str = rel.to_string_lossy().to_string();

    // Convert backslashes to forward slashes (Windows)
    path_str = path_str.replace('\\', "/");

    // Remove known extensions
    for ext in &[".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"] {
        if path_str.ends_with(ext) {
            path_str = path_str[..path_str.len() - ext.len()].to_string();
            break;
        }
    }

    // Remove /index suffix (barrel imports)
    if path_str.ends_with("/index") {
        path_str = path_str[..path_str.len() - 6].to_string();
    }

    // Ensure "./" prefix for relative paths
    if !path_str.starts_with('.') && !path_str.starts_with('/') {
        path_str = format!("./{path_str}");
    }

    path_str
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relative_path_basic() {
        let test_file = Path::new("/project/src/services/__tests__/user.test.ts");
        let target = Path::new("/project/src/repositories/user.repository.ts");
        assert_eq!(
            compute_relative_path(test_file, target),
            "../../repositories/user.repository"
        );
    }

    #[test]
    fn test_relative_path_same_dir() {
        let test_file = Path::new("/project/src/utils/foo.test.ts");
        let target = Path::new("/project/src/utils/bar.ts");
        assert_eq!(compute_relative_path(test_file, target), "./bar");
    }

    #[test]
    fn test_relative_path_index() {
        let test_file = Path::new("/project/src/services/user.test.ts");
        let target = Path::new("/project/src/lib/index.ts");
        assert_eq!(compute_relative_path(test_file, target), "../lib");
    }
}
