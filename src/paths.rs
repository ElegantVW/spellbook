//! Location bar helpers. Fixtures never use the operator's home.

use std::path::{Component, Path, PathBuf};

pub fn path_crumbs(path: &Path) -> Vec<(String, PathBuf)> {
    let path = if path.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        path.to_path_buf()
    };
    let mut crumbs: Vec<(String, PathBuf)> = Vec::new();
    let mut acc = PathBuf::new();
    let mut any = false;
    for c in path.components() {
        any = true;
        match c {
            Component::RootDir => {
                acc = PathBuf::from("/");
                crumbs.push(("/".into(), acc.clone()));
            }
            Component::Prefix(p) => {
                acc = PathBuf::from(p.as_os_str());
                crumbs.push((p.as_os_str().to_string_lossy().into(), acc.clone()));
            }
            Component::Normal(s) => {
                acc.push(s);
                crumbs.push((s.to_string_lossy().into(), acc.clone()));
            }
            Component::CurDir => {
                if crumbs.is_empty() {
                    acc = PathBuf::from(".");
                    crumbs.push((".".into(), acc.clone()));
                }
            }
            Component::ParentDir => {
                acc.push("..");
                crumbs.push(("..".into(), acc.clone()));
            }
        }
    }
    if !any {
        crumbs.push((".".into(), PathBuf::from(".")));
    }
    crumbs
}

pub fn clean_pasted_path(typed: &str) -> String {
    let raw = typed.replace("\r\n", "\n").replace('\r', "\n");
    let mut raw = raw.split('\n').next().unwrap_or("").trim().to_string();
    let b = raw.as_bytes();
    if b.len() >= 2 && b[0] == b[b.len() - 1] && (b[0] == b'\'' || b[0] == b'"') {
        raw = raw[1..raw.len() - 1].trim().to_string();
    }
    raw
}

pub fn expand_user(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        home().join(rest)
    } else if path == "~" {
        home()
    } else {
        PathBuf::from(path)
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Map a typed/pasted location to (dir, select_file, error).
pub fn resolve_goto(
    current: &Path,
    typed: &str,
) -> Result<(PathBuf, Option<PathBuf>), &'static str> {
    let raw = clean_pasted_path(typed);
    if raw.is_empty() {
        return Err("empty");
    }
    let mut p = expand_user(&raw);
    if !p.is_absolute() {
        p = current.join(p);
    }
    let p = p.canonicalize().unwrap_or(p);
    if p.is_dir() {
        return Ok((p, None));
    }
    if p.exists() {
        let parent = p.parent().unwrap_or(current).to_path_buf();
        return Ok((parent, Some(p)));
    }
    Err("missing")
}

/// Ctrl-W: delete the path component before the caret.
pub fn delete_component(buf: &str, caret: usize) -> (String, usize) {
    let caret = caret.min(buf.len());
    let bytes = buf.as_bytes();
    let mut i = caret;
    while i > 0 && bytes[i - 1] == b'/' {
        i -= 1;
    }
    while i > 0 && bytes[i - 1] != b'/' {
        i -= 1;
    }
    let mut out = String::new();
    out.push_str(&buf[..i]);
    out.push_str(&buf[caret..]);
    (out, i)
}

pub fn write_pick(chosen: &Path, output: Option<&Path>) -> std::io::Result<()> {
    let s = chosen.display().to_string();
    if let Some(p) = output {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        std::fs::write(p, s)
    } else {
        println!("{s}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn path_crumbs_unix_root() {
        let crumbs = path_crumbs(Path::new("/"));
        assert_eq!(crumbs.len(), 1);
        assert_eq!(crumbs[0].0, "/");
        assert_eq!(crumbs[0].1, PathBuf::from("/"));
    }

    #[test]
    fn path_crumbs_nested() {
        let crumbs = path_crumbs(Path::new("/srv/faeos/bin"));
        let labels: Vec<_> = crumbs.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, vec!["/", "srv", "faeos", "bin"]);
        assert_eq!(crumbs.last().unwrap().1, PathBuf::from("/srv/faeos/bin"));
        assert_eq!(crumbs[1].1, PathBuf::from("/srv"));
    }

    #[test]
    fn clean_pasted_path_quotes_and_newline() {
        assert_eq!(clean_pasted_path("  \"/tmp/foo\"\nextra\n"), "/tmp/foo");
        assert_eq!(clean_pasted_path("'/tmp/bar'"), "/tmp/bar");
        assert_eq!(clean_pasted_path(""), "");
    }

    #[test]
    fn resolve_goto_dir() {
        let tmp = std::env::temp_dir().join(format!("sb-goto-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let (dest, select) = resolve_goto(&tmp, tmp.to_str().unwrap()).unwrap();
        assert_eq!(dest, tmp.canonicalize().unwrap());
        assert!(select.is_none());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolve_goto_file_selects_parent() {
        let tmp = std::env::temp_dir().join(format!("sb-file-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let f = tmp.join("scroll.txt");
        fs::write(&f, "hi").unwrap();
        let (dest, select) = resolve_goto(&tmp, f.to_str().unwrap()).unwrap();
        assert_eq!(dest, tmp.canonicalize().unwrap());
        assert_eq!(select.unwrap().file_name().unwrap(), "scroll.txt");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolve_goto_relative_and_missing() {
        let tmp = std::env::temp_dir().join(format!("sb-rel-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let nested = tmp.join("tomes");
        fs::create_dir_all(&nested).unwrap();
        let (dest, _) = resolve_goto(&tmp, "tomes").unwrap();
        assert_eq!(dest, nested.canonicalize().unwrap());
        assert!(resolve_goto(&tmp, "no-such-shelf").is_err());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn resolve_goto_tilde() {
        let tmp = std::env::temp_dir().join(format!("sb-home-{}", std::process::id()));
        let docs = tmp.join("docs");
        fs::create_dir_all(&docs).unwrap();
        let old = std::env::var("HOME").ok();
        std::env::set_var("HOME", &tmp);
        let r = resolve_goto(Path::new("/"), "~/docs");
        if let Some(h) = old {
            std::env::set_var("HOME", h);
        }
        let (dest, _) = r.unwrap();
        assert_eq!(dest, docs.canonicalize().unwrap());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ctrl_w_deletes_path_component() {
        let (s, i) = delete_component("/srv/faeos/bin", "/srv/faeos/bin".len());
        assert_eq!(s, "/srv/faeos/");
        assert_eq!(i, "/srv/faeos/".len());
    }

    #[test]
    fn write_pick_to_file() {
        let tmp = std::env::temp_dir().join(format!("sb-pick-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let out = tmp.join("chosen.path");
        write_pick(Path::new("/srv/faeos/bin"), Some(&out)).unwrap();
        assert_eq!(fs::read_to_string(&out).unwrap(), "/srv/faeos/bin");
        let _ = fs::remove_dir_all(&tmp);
    }
}
