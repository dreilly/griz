use std::fs;
use std::path::{Path, PathBuf};

pub fn workspace_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    let length = name.chars().count();
    if !(1..=80).contains(&length) {
        return Err("workspace name must be 1 to 80 characters after trimming".into());
    }
    if name.chars().any(char::is_control) {
        return Err("workspace name cannot contain control characters".into());
    }
    Ok(name.to_owned())
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
