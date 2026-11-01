//! Cached directory listing. Reload only when the shelf changes.

use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Name,
    Date,
    Size,
    Type,
}

impl Sort {
    pub fn as_str(self) -> &'static str {
        match self {
            Sort::Name => "name",
            Sort::Date => "date",
            Sort::Size => "size",
            Sort::Type => "type",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Sort::Name => Sort::Date,
            Sort::Date => Sort::Size,
            Sort::Size => Sort::Type,
            Sort::Type => Sort::Name,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dir,
    Link,
    Exec,
    Image,
    Video,
    Audio,
    Archive,
    Markdown,
    Markup,
    Pdf,
    Code,
    Text,
}

impl Kind {
    pub fn tag(self) -> &'static str {
        match self {
            Kind::Dir => "[D]",
            Kind::Link => "[L]",
            Kind::Exec => "[x]",
            Kind::Image => "[I]",
            Kind::Video => "[V]",
            Kind::Audio => "[A]",
            Kind::Archive => "[Z]",
            Kind::Markdown => "[M]",
            Kind::Markup => "[<]",
            Kind::Pdf => "[P]",
            Kind::Code => "[C]",
            Kind::Text => "[F]",
        }
    }

    pub fn from_name(name: &str, is_dir: bool, is_link: bool, executable: bool) -> Self {
        if is_dir {
            return Kind::Dir;
        }
        if is_link {
            return Kind::Link;
        }
        if executable {
            return Kind::Exec;
        }
        let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" => Kind::Image,
            "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "mpg" | "mpeg" | "3gp" => Kind::Video,
            "flac" | "mp3" | "ogg" | "wav" | "m4a" | "opus" => Kind::Audio,
            "zip" | "tar" | "gz" | "tgz" | "7z" | "xz" | "bz2" | "rar" => Kind::Archive,
            "md" | "markdown" | "mdown" | "mkd" => Kind::Markdown,
            "html" | "htm" | "xhtml" | "xml" | "svg" => Kind::Markup,
            "pdf" => Kind::Pdf,
            "rs" | "py" | "c" | "h" | "cc" | "hh" | "js" | "ts" | "tsx" | "jsx" | "go" | "toml"
            | "json" | "yaml" | "yml" | "sh" | "zsh" | "bash" | "lua" | "rb" | "java" | "kt"
            | "css" | "rsx" => Kind::Code,
            _ => Kind::Text,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    #[allow(dead_code)]
    pub is_link: bool,
    #[allow(dead_code)]
    pub executable: bool,
    pub kind: Kind,
    pub size: u64,
    pub mtime: SystemTime,
}

impl Entry {
    pub fn size_label(&self) -> String {
        if self.is_dir {
            String::new()
        } else {
            format_size(self.size)
        }
    }
}

pub fn format_size(size: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut n = size as f64;
    for unit in units {
        if n < 1024.0 {
            return format!("{n:.1} {unit}");
        }
        n /= 1024.0;
    }
    format!("{n:.1} PB")
}

#[derive(Clone, Debug)]
pub struct Listing {
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    pub sort: Sort,
    pub hidden: bool,
    pub error: Option<String>,
}

impl Listing {
    pub fn open(dir: PathBuf, sort: Sort, hidden: bool) -> Self {
        let mut s = Self {
            dir,
            entries: Vec::new(),
            sort,
            hidden,
            error: None,
        };
        s.reload();
        s
    }

    pub fn reload(&mut self) {
        self.error = None;
        match fs::read_dir(&self.dir) {
            Err(e) => {
                self.entries.clear();
                self.error = Some(e.to_string());
            }
            Ok(rd) => {
                let mut entries = Vec::new();
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name == "." || name == ".." {
                        continue;
                    }
                    if !self.hidden && name.starts_with('.') {
                        continue;
                    }
                    let p = e.path();
                    let meta = e.metadata().ok();
                    let is_link = meta.as_ref().map(|m| m.file_type().is_symlink()).unwrap_or(false)
                        || p.symlink_metadata()
                            .map(|m| m.file_type().is_symlink())
                            .unwrap_or(false);
                    let is_dir = p.is_dir();
                    let executable = !is_dir && unix_exec(meta.as_ref());
                    let kind = Kind::from_name(&name, is_dir, is_link, executable);
                    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                    let mtime = meta
                        .and_then(|m| m.modified().ok())
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    entries.push(Entry {
                        path: p,
                        name,
                        is_dir,
                        is_link,
                        executable,
                        kind,
                        size,
                        mtime,
                    });
                }
                match self.sort {
                    Sort::Name => entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
                    Sort::Date => entries.sort_by(|a, b| b.mtime.cmp(&a.mtime)),
                    Sort::Size => entries.sort_by(|a, b| b.size.cmp(&a.size)),
                    Sort::Type => entries.sort_by(|a, b| {
                        (!a.is_dir, a.name.to_lowercase()).cmp(&(!b.is_dir, b.name.to_lowercase()))
                    }),
                }
                self.entries = entries;
            }
        }
    }

    pub fn select_name(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.name == name)
    }
}

fn unix_exec(meta: Option<&fs::Metadata>) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn lists_fixture_without_hidden() {
        let tmp = std::env::temp_dir().join(format!("sb-list-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("note.md"), "x").unwrap();
        fs::create_dir(tmp.join("alpha")).unwrap();
        fs::write(tmp.join(".veil"), "").unwrap();
        let listing = Listing::open(tmp.clone(), Sort::Name, false);
        let names: Vec<_> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"note.md"), "{names:?}");
        assert!(names.contains(&"alpha"), "{names:?}");
        assert!(!names.contains(&".veil"), "{names:?}");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn kind_from_suffix() {
        assert_eq!(Kind::from_name("pic.png", false, false, false), Kind::Image);
        assert_eq!(Kind::from_name("song.flac", false, false, false), Kind::Audio);
        assert_eq!(Kind::from_name("main.rs", false, false, false), Kind::Code);
        assert_eq!(Kind::from_name("pack.zip", false, false, false), Kind::Archive);
        assert_eq!(Kind::from_name("notes", true, false, false), Kind::Dir);
        assert_eq!(Kind::from_name("bin", false, false, true), Kind::Exec);
        assert_eq!(Kind::from_name("link", false, true, false), Kind::Link);
        assert_eq!(
            Kind::from_name("clip.webm", false, false, false),
            Kind::Video
        );
        assert_eq!(
            Kind::from_name("guide.MD", false, false, false),
            Kind::Markdown
        );
        assert_eq!(
            Kind::from_name("page.HTML", false, false, false),
            Kind::Markup
        );
        assert_eq!(
            Kind::from_name("manual.pdf", false, false, false),
            Kind::Pdf
        );
    }
}
