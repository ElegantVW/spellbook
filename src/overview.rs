use crate::listing::{Entry, Kind};
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::Path;
use std::time::SystemTime;

const PREVIEW_LIMIT: usize = 4096;

pub struct Overview {
    pub name: String,
    pub path: String,
    pub kind: &'static str,
    pub size: String,
    pub modified: String,
    pub permissions: String,
    pub preview: Preview,
}

pub enum Preview {
    Text { content: String, truncated: bool },
    Message(String),
}

pub fn inspect(entry: &Entry) -> Overview {
    let path = entry.path.display().to_string();
    let metadata = match fs::symlink_metadata(&entry.path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return Overview {
                name: entry.name.clone(),
                path,
                kind: kind_label(entry.kind),
                size: "Unavailable".into(),
                modified: "Unavailable".into(),
                permissions: "Unavailable".into(),
                preview: Preview::Message(format!("Could not inspect entry: {error}")),
            };
        }
    };

    let is_link = metadata.file_type().is_symlink();
    let kind = if is_link {
        Kind::Link
    } else if metadata.is_dir() {
        Kind::Dir
    } else {
        entry.kind
    };
    let preview = if is_link {
        Preview::Message("Symbolic link; preview is disabled.".into())
    } else if metadata.is_dir() {
        Preview::Message("Directory; no file preview.".into())
    } else if !metadata.is_file() {
        Preview::Message("Special file; preview is disabled.".into())
    } else {
        read_preview(&entry.path)
    };

    Overview {
        name: entry.name.clone(),
        path,
        kind: kind_label(kind),
        size: if metadata.is_file() {
            crate::listing::format_size(metadata.len())
        } else {
            "—".into()
        },
        modified: metadata
            .modified()
            .map(format_age)
            .unwrap_or_else(|error| format!("Unavailable: {error}")),
        permissions: permissions(&metadata),
        preview,
    }
}

fn read_preview(path: &Path) -> Preview {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }

    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) => {
            return Preview::Message(format!("Could not open preview: {error}"));
        }
    };
    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            return Preview::Message(format!("Could not inspect preview: {error}"));
        }
    };
    if !metadata.is_file() {
        return Preview::Message("Not a regular file; preview is disabled.".into());
    }

    let mut bytes = Vec::with_capacity(PREVIEW_LIMIT + 1);
    if let Err(error) = file
        .by_ref()
        .take((PREVIEW_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
    {
        return Preview::Message(format!("Could not read preview: {error}"));
    }
    let truncated = bytes.len() > PREVIEW_LIMIT;
    if bytes.contains(&0) {
        return Preview::Message("Binary file; preview is disabled.".into());
    }
    bytes.truncate(PREVIEW_LIMIT);

    let content = match std::str::from_utf8(&bytes) {
        Ok(content) => content.to_string(),
        Err(error) if truncated && error.error_len().is_none() => {
            match std::str::from_utf8(&bytes[..error.valid_up_to()]) {
                Ok(content) => content.to_string(),
                Err(_) => {
                    return Preview::Message(
                        "Binary or non-UTF-8 file; preview is disabled.".into(),
                    );
                }
            }
        }
        Err(_) => {
            return Preview::Message("Binary or non-UTF-8 file; preview is disabled.".into());
        }
    };

    Preview::Text {
        content: sanitize(&content),
        truncated,
    }
}

fn sanitize(content: &str) -> String {
    content
        .chars()
        .map(|character| {
            if character == '\t' {
                "    ".into()
            } else if character.is_control() && character != '\n' {
                format!("\\u{{{:x}}}", character as u32)
            } else {
                character.to_string()
            }
        })
        .collect()
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Dir => "Directory",
        Kind::Link => "Symbolic link",
        Kind::Exec => "Executable",
        Kind::Image => "Image",
        Kind::Audio => "Audio",
        Kind::Archive => "Archive",
        Kind::Code => "Code / text",
        Kind::Text => "File",
    }
}

fn format_age(modified: SystemTime) -> String {
    let Ok(age) = SystemTime::now().duration_since(modified) else {
        return "in the future".into();
    };
    let seconds = age.as_secs();
    if seconds < 60 {
        format!("{seconds}s ago")
    } else if seconds < 3600 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h ago", seconds / 3600)
    } else {
        format!("{}d ago", seconds / 86_400)
    }
}

#[cfg(unix)]
fn permissions(metadata: &fs::Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;
    format!("{:04o}", metadata.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn permissions(metadata: &fs::Metadata) -> String {
    if metadata.permissions().readonly() {
        "read-only".into()
    } else {
        "read/write".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listing::{Listing, Sort};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("spellbook-overview-{}-{nonce}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn entry(&self, name: &str) -> Entry {
            Listing::open(self.0.clone(), Sort::Name, true)
                .entries
                .into_iter()
                .find(|entry| entry.name == name)
                .unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn previews_text_with_a_hard_byte_limit() {
        let fixture = Fixture::new();
        let contents = format!("{}x", "a".repeat(PREVIEW_LIMIT));
        fs::write(fixture.0.join("large.txt"), contents).unwrap();

        match inspect(&fixture.entry("large.txt")).preview {
            Preview::Text { content, truncated } => {
                assert_eq!(content.len(), PREVIEW_LIMIT);
                assert!(truncated);
            }
            Preview::Message(message) => panic!("unexpected preview message: {message}"),
        }
    }

    #[test]
    fn previews_empty_text_files() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("empty.txt"), []).unwrap();

        match inspect(&fixture.entry("empty.txt")).preview {
            Preview::Text { content, truncated } => {
                assert!(content.is_empty());
                assert!(!truncated);
            }
            Preview::Message(message) => panic!("unexpected preview message: {message}"),
        }
    }

    #[test]
    fn escapes_terminal_control_characters_in_text() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("controls.txt"), "hello\u{1b}[31m").unwrap();

        match inspect(&fixture.entry("controls.txt")).preview {
            Preview::Text { content, .. } => {
                assert_eq!(content, "hello\\u{1b}[31m");
            }
            Preview::Message(message) => panic!("unexpected preview message: {message}"),
        }
    }

    #[test]
    fn does_not_preview_binary_or_special_files() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("binary.dat"), [b'a', 0, b'b']).unwrap();
        fs::create_dir(fixture.0.join("folder")).unwrap();

        assert!(matches!(
            inspect(&fixture.entry("binary.dat")).preview,
            Preview::Message(message) if message.contains("Binary")
        ));
        assert!(matches!(
            inspect(&fixture.entry("folder")).preview,
            Preview::Message(message) if message.contains("Directory")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_symbolic_links_for_preview() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        fs::write(fixture.0.join("secret.txt"), "not previewed").unwrap();
        symlink(fixture.0.join("secret.txt"), fixture.0.join("link.txt")).unwrap();

        assert!(matches!(
            inspect(&fixture.entry("link.txt")).preview,
            Preview::Message(message) if message.contains("Symbolic link")
        ));
    }

    #[test]
    fn missing_file_errors_are_shown() {
        let fixture = Fixture::new();
        let missing = fixture.0.join("missing.txt");
        assert!(matches!(
            read_preview(&missing),
            Preview::Message(message) if message.contains("Could not open preview")
        ));
    }
}
