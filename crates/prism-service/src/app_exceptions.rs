//! User-selected executable identities; never execute a submitted path.
use anyhow::{ensure, Result};
#[cfg(windows)]
use anyhow::Context;

pub const MAX_APPS: usize = 32;

pub fn local_exe_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 7 && bytes.len() <= 1024
        && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
        && !path[2..].contains(':')
        && !path.chars().any(|c| c.is_control() || matches!(c, '*' | '?' | '"' | '<' | '>' | '|'))
        && path.to_ascii_lowercase().ends_with(".exe")
}

pub fn validate(paths: &[String]) -> Result<Vec<String>> {
    ensure!(paths.len() <= MAX_APPS, "断网保护最多允许 32 个应用例外");
    if paths.is_empty() { return Ok(Vec::new()); }
    #[cfg(not(windows))]
    anyhow::bail!("断网保护应用例外目前只支持 Windows");
    #[cfg(windows)] {
        let mut result = Vec::<String>::new();
        for raw in paths {
            let input = raw.trim().trim_matches('"');
            ensure!(local_exe_path(input), "请选择本地磁盘上 .exe 文件的完整路径，不支持目录、通配符或网络路径");
            let path = std::fs::canonicalize(input).context("应用文件不存在或无法访问")?;
            let canonical = path.to_str().context("应用路径不是有效的 Unicode")?;
            let canonical = canonical.strip_prefix(r"\\?\").unwrap_or(canonical);
            ensure!(local_exe_path(canonical) && path.is_file(), "应用例外必须指向本地 .exe 文件");
            if !result.iter().any(|p| p.eq_ignore_ascii_case(canonical)) {
                result.push(canonical.to_string());
            }
        }
        Ok(result)
    }
}

/// App uninstall/update must not prevent the proxy core starting. Missing
/// executables get no permission; explicit UI saves still validate strictly.
pub fn available(paths: &[String]) -> Result<Vec<String>> {
    ensure!(paths.len() <= MAX_APPS, "断网保护最多允许 32 个应用例外");
    #[cfg(windows)] {
        let mut result = Vec::new();
        for raw in paths {
            let input = raw.trim().trim_matches('"');
            ensure!(local_exe_path(input), "应用例外路径格式无效");
            if std::path::Path::new(input).is_file() {
                result.extend(validate(&[input.into()])?);
            }
        }
        validate(&result)
    }
    #[cfg(not(windows))]
    validate(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_broad_or_nonlocal_identities() {
        for path in ["app.exe", r"C:app.exe", r"\\server\share\app.exe", r"\\?\C:\app.exe", r"C:\Apps\*.exe", r"C:\Apps\app.exe:stream", r"C:\Apps", r"C:\Apps\run.ps1"] {
            assert!(!local_exe_path(path), "{path}");
        }
        assert!(local_exe_path(r"C:\Program Files\Remote App\app.EXE"));
        assert!(validate(&[]).unwrap().is_empty());
        assert!(validate(&vec!["app.exe".into(); MAX_APPS + 1]).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn validates_existing_files_and_deduplicates_case() {
        let path = std::env::current_exe().unwrap().to_string_lossy().into_owned();
        let paths = validate(&[path.clone(), path.to_uppercase()]).unwrap();
        assert_eq!(paths.len(), 1);
        assert!(validate(&[r"C:\definitely-missing-prism-test\app.exe".into()]).is_err());
        assert!(available(&[r"C:\definitely-missing-prism-test\app.exe".into()]).unwrap().is_empty());
    }
}
