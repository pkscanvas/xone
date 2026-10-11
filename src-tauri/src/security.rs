use std::path::Path;

/// Only web pages may be handed to the operating system from feed content.
pub fn browser_url(value: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(value.trim())
        .map_err(|_| "This article does not have a valid web address".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("Only HTTP and HTTPS article links can be opened".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Article links must not contain credentials".to_string());
    }
    Ok(url.to_string())
}

pub fn private_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(path)?;
    Ok(())
}

pub fn private_database(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        // Existing journal files can contain the same sensitive settings.
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            match std::fs::set_permissions(Path::new(&name), std::fs::Permissions::from_mode(0o600))
            {
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                result => result?,
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_article_links_and_rejects_os_handlers() {
        for link in [
            "https://example.com/story?a=1#part",
            "http://localhost:1234/news",
            "https://例え.jp/news",
        ] {
            assert!(browser_url(link).is_ok());
        }
        for link in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "mailto:test@example.com",
            "xone://settings",
            "/Applications/Calculator.app",
            "https://user:password@example.com/",
        ] {
            assert!(browser_url(link).is_err(), "must reject {link}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn restricts_existing_data_without_changing_contents() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("xone-permissions-{}", std::process::id()));
        private_directory(&dir).unwrap();
        let db = dir.join("settings.sqlite3");
        std::fs::write(&db, b"existing settings").unwrap();
        std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o644)).unwrap();
        private_database(&db).unwrap();
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&db).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(std::fs::read(&db).unwrap(), b"existing settings");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
