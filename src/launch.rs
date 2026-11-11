//! Launch policy for the bundled GUI, independent of Finder's cwd/environment.
use std::ffi::{CStr, OsStr};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn is_app_bundle(executable: &Path) -> bool {
    executable.parent().is_some_and(|macos| {
        macos.file_name() == Some(OsStr::new("MacOS"))
            && macos.parent().is_some_and(|contents| {
                contents.file_name() == Some(OsStr::new("Contents"))
                    && contents
                        .parent()
                        .is_some_and(|app| app.extension() == Some(OsStr::new("app")))
            })
    })
}

pub(crate) fn core_command(executable: &Path) -> io::Result<Command> {
    let mut command = Command::new(executable.with_file_name("griz-core"));
    if is_app_bundle(executable) {
        let (home, shell) = login_account()?;
        configure_bundle(&mut command, &home, &shell)?;
    }
    Ok(command)
}

fn configure_bundle(command: &mut Command, home: &Path, shell: &Path) -> io::Result<()> {
    if !home.is_absolute() {
        return Err(io::Error::other(
            "login account has no absolute home directory",
        ));
    }
    // Finder can launch with cwd=/ and without SHELL. Use the account database,
    // not the build machine's paths or a guessed /Users/<name> directory.
    command.current_dir(home).env("HOME", home).env(
        "SHELL",
        if shell.is_absolute() {
            shell
        } else {
            Path::new("/bin/sh")
        },
    );
    Ok(())
}

fn login_account() -> io::Result<(PathBuf, PathBuf)> {
    let mut buffer = vec![0_u8; 16 * 1024];
    loop {
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        // SAFETY: entry, result, and buffer point to writable storage for the
        // call. On success the record's strings live in buffer until copied.
        let error = unsafe {
            libc::getpwuid_r(
                libc::geteuid(),
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if error == libc::ERANGE && buffer.len() < 1024 * 1024 {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
        if result.is_null() {
            return Err(io::Error::other(
                "could not resolve the current login account",
            ));
        }
        // SAFETY: getpwuid_r succeeded with a non-null result, initializing
        // entry and its NUL-terminated strings within the still-live buffer.
        let entry = unsafe { entry.assume_init() };
        let path = |ptr: *const libc::c_char| {
            if ptr.is_null() {
                PathBuf::new()
            } else {
                // SAFETY: this pointer is a passwd string from the successful call.
                PathBuf::from(OsStr::from_bytes(unsafe { CStr::from_ptr(ptr) }.to_bytes()))
            }
        };
        return Ok((path(entry.pw_dir), path(entry.pw_shell)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_installed_and_translocated_app_bundles_only() {
        for path in [
            "/Applications/Griz.app/Contents/MacOS/griz",
            "/private/var/folders/random/AppTranslocation/id/d/Griz.app/Contents/MacOS/griz",
            "/Users/test/Downloads/A folder/Griz.app/Contents/MacOS/griz",
        ] {
            assert!(is_app_bundle(Path::new(path)));
        }
        for path in [
            "/code/griz/target/debug/griz",
            "/code/griz/target/release/griz",
            "/tmp/Contents/MacOS/griz",
            "/Applications/Griz.app/griz",
        ] {
            assert!(!is_app_bundle(Path::new(path)));
        }
    }

    #[test]
    fn bundled_launch_uses_home_and_login_shell_without_shell_expansion() {
        let mut command = Command::new("griz-core");
        let home = Path::new("/Users/Test Person");
        configure_bundle(&mut command, home, Path::new("/bin/zsh")).unwrap();
        assert_eq!(command.get_current_dir(), Some(home));
        let env: Vec<_> = command.get_envs().collect();
        assert!(env.contains(&(OsStr::new("HOME"), Some(home.as_os_str()))));
        assert!(env.contains(&(OsStr::new("SHELL"), Some(OsStr::new("/bin/zsh")))));
        assert_eq!(command.get_args().count(), 0);
    }

    #[test]
    fn development_launch_preserves_the_callers_directory_and_environment() {
        let command = core_command(Path::new("/code/griz/target/debug/griz")).unwrap();
        assert_eq!(command.get_program(), "/code/griz/target/debug/griz-core");
        assert!(command.get_current_dir().is_none());
        assert_eq!(command.get_envs().count(), 0);
    }

    #[test]
    fn missing_home_is_an_error_and_missing_shell_has_a_safe_fallback() {
        let mut command = Command::new("griz-core");
        assert!(configure_bundle(&mut command, Path::new(""), Path::new("/bin/zsh")).is_err());
        configure_bundle(&mut command, Path::new("/Users/test"), Path::new("")).unwrap();
        assert!(
            command
                .get_envs()
                .any(|(key, value)| { key == "SHELL" && value == Some(OsStr::new("/bin/sh")) })
        );
    }

    #[test]
    fn resolves_the_current_accounts_home_and_shell() {
        let (home, shell) = login_account().unwrap();
        assert!(home.is_absolute() && home.is_dir());
        assert!(shell.is_absolute());
    }
}
