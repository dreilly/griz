use std::fs;
use std::path::{Path, PathBuf};

pub fn workspace_name(name: &str) -> Result<Option<String>, String> {
    let name = name.trim();
    if name.is_empty() {
        return Ok(None);
    }
    let length = name.chars().count();
    if length > 80 {
        return Err("custom workspace name must be at most 80 characters after trimming".into());
    }
    if name.chars().any(char::is_control) {
        return Err("workspace name cannot contain control characters".into());
    }
    Ok(Some(name.to_owned()))
}

pub fn directory_label(cwd: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    directory_basename(cwd, home.as_deref())
}

fn directory_basename(cwd: &Path, home: Option<&Path>) -> String {
    if home == Some(cwd) {
        return "~".into();
    }
    cwd.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.to_string_lossy().into_owned())
}

pub fn directory_path(cwd: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    directory_name(cwd, home.as_deref())
}

/// Only abbreviate a real home-directory prefix, not a similarly named sibling.
fn directory_name(cwd: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home.filter(|path| path.is_absolute())
        && let Ok(relative) = cwd.strip_prefix(home)
    {
        return if relative.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~/{}", relative.display())
        };
    }
    cwd.to_string_lossy().into_owned()
}

pub fn validate_cwd(cwd: Option<&str>, default: &Path) -> Result<PathBuf, String> {
    let path = cwd.map(PathBuf::from).unwrap_or_else(|| default.to_owned());
    if !path.is_absolute() {
        return Err("terminal directory must be an absolute path".into());
    }
    let metadata = fs::metadata(&path)
        .map_err(|error| format!("terminal directory {}: {error}", path.display()))?;
    if !metadata.is_dir() {
        return Err(format!(
            "terminal directory {} is not a directory",
            path.display()
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_names_use_tilde_for_home_and_descendants_only() {
        let home = Some(Path::new("/Users/daniel"));
        for (path, expected) in [
            ("/Users/daniel", "~"),
            ("/Users/daniel/code", "~/code"),
            (
                "/Users/daniel/code/my project/日本語",
                "~/code/my project/日本語",
            ),
            ("/Users/daniel-other/code", "/Users/daniel-other/code"),
            ("/opt/project", "/opt/project"),
            ("/", "/"),
        ] {
            assert_eq!(directory_name(Path::new(path), home), expected);
        }
        assert_eq!(
            directory_name(Path::new("/opt/project"), None),
            "/opt/project"
        );
    }

    #[test]
    fn workspace_names_are_optional_but_custom_names_are_still_validated() {
        assert_eq!(workspace_name("").unwrap(), None);
        assert_eq!(workspace_name("   ").unwrap(), None);
        assert_eq!(
            workspace_name("  My workspace  ").unwrap(),
            Some("My workspace".into())
        );
        assert!(workspace_name(&"x".repeat(81)).is_err());
        assert!(workspace_name("bad\u{7}name").is_err());
    }

    #[test]
    fn directory_labels_show_only_the_basename_with_home_and_root_special_cases() {
        let home = Some(Path::new("/Users/daniel"));
        for (path, label) in [
            ("/Users/daniel", "~"),
            ("/Users/daniel/code/griz", "griz"),
            ("/opt/code/", "code"),
            ("/", "/"),
            ("/opt/my project/日本語", "日本語"),
        ] {
            assert_eq!(directory_basename(Path::new(path), home), label);
        }
        assert_eq!(
            directory_basename(Path::new("/opt/project"), None),
            "project"
        );
        // Automatic labels are not restricted by the custom-name length limit.
        let long_name = "x".repeat(100);
        assert_eq!(
            directory_label(&PathBuf::from(format!("/opt/{long_name}"))),
            long_name
        );
    }
}
