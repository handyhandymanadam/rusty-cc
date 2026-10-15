//! Path tidying for records: `/usr/bin/../lib/gcc/x86_64-redhat-linux/16/../../../../include/c++/16/bits/x.h` -> `/usr/include/c++/16/bits/x.h`.
use std::path::{Component, Path, PathBuf};

/// The same place without `.` and `..` parts. Lexical, but only used when the result exists too (or the original does not):
/// a path through a symbolic link keeps its original form.
pub fn tidy(p: &str) -> String {
    if !(p.contains("/../") || p.contains("/./") || p.contains("//") || p.ends_with("/..") || p.ends_with("/.")) { return p.to_string(); }
    let mut out = PathBuf::new();
    for c in Path::new(p).components() {
        match c {
            Component::ParentDir => { if !out.pop() { out.push(".."); } }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    let t = out.to_string_lossy().into_owned();
    if t.is_empty() { return p.to_string(); }
    if Path::new(p).exists() && !Path::new(&t).exists() { return p.to_string(); }
    t
}

/// "path:12:3: message" with its path tidied; text that does not start with a path is returned unchanged
pub fn tidy_prefixed(s: &str) -> String {
    if !s.starts_with('/') { return s.to_string(); }
    let b = s.as_bytes();
    for i in 1..b.len().saturating_sub(1) { if b[i] == b':' && b[i + 1].is_ascii_digit() { return format!("{}{}", tidy(&s[..i]), &s[i..]); } }
    s.to_string()
}

/// a path as short as it can be: relative to the current directory when it lies under it, otherwise tidied
pub fn short(p: &str, cwd: Option<&str>) -> String {
    let t = tidy(p);
    match cwd { Some(c) if t.starts_with('/') => match Path::new(&t).strip_prefix(c) { Ok(r) if !r.as_os_str().is_empty() => r.to_string_lossy().into_owned(), _ => t }, _ => t }
}
