//! File types by family, from the lower-case extension: shared by the views (bars grouped by
//! family) and the advisor (what in a folder is the user's own work).

pub const IMAGES: u8 = 0;
pub const VIDEO: u8 = 1;
pub const AUDIO: u8 = 2;
pub const ARCHIVES: u8 = 3;
pub const PROGRAMS: u8 = 4;
pub const DOCUMENTS: u8 = 5;
pub const CODE: u8 = 6;
pub const OTHER: u8 = 7;

/// The lower-case extensions of each family but [`OTHER`] (Search's Type filter offers them).
pub const FAMILIES: [(u8, &[&str]); 7] = [
    (IMAGES, &["jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "tif", "tiff", "raw", "cr2", "nef", "arw", "dng", "psd", "svg", "ico"]),
    (VIDEO, &["mp4", "mkv", "mov", "avi", "wmv", "webm", "m4v", "ts"]),
    (AUDIO, &["mp3", "wav", "flac", "aac", "ogg", "m4a", "wma"]),
    (ARCHIVES, &["zip", "7z", "rar", "gz", "xz", "tar", "cab", "msi", "iso", "zst", "bz2"]),
    (PROGRAMS, &["exe", "dll", "sys", "so", "lib", "a", "node", "pyd", "jar", "class"]),
    (DOCUMENTS, &["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "rtf", "odt"]),
    (CODE, &["js", "ts2", "py", "rs", "c", "cpp", "h", "cs", "java", "go", "json", "xml", "html", "css", "yml", "yaml", "toml"]),
];

/// A file type's family.
pub fn family(ext: &str) -> u8 {
    FAMILIES.iter().find(|(_, exts)| exts.contains(&ext)).map_or(OTHER, |f| f.0)
}

/// The extensions of family `f` (none for [`OTHER`]).
pub fn extensions(f: u8) -> &'static [&'static str] {
    FAMILIES.iter().find(|x| x.0 == f).map_or(&[], |x| x.1)
}

/// A file the user made rather than a program: a document, an image project, source code.
/// Text, markdown and the formats programs keep their settings in (json, xml, yaml, toml, ini,
/// html, js) are left out: every application writes those.
pub fn user_work(ext: &str) -> bool {
    match family(ext) {
        DOCUMENTS => !matches!(ext, "txt" | "md"),
        IMAGES => ext == "psd",
        CODE => !matches!(ext, "json" | "xml" | "html" | "css" | "yml" | "yaml" | "toml" | "js"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_is_documents_projects_and_source() {
        for e in ["docx", "pdf", "xlsx", "psd", "rs", "py", "cpp"] {
            assert!(user_work(e), "{e}");
        }
        for e in ["json", "xml", "txt", "ini", "log", "png", "dll", "js", "toml"] {
            assert!(!user_work(e), "{e}");
        }
        assert_eq!(family("mp4"), VIDEO);
    }

    #[test]
    fn families_and_their_lists_agree() {
        for f in [IMAGES, VIDEO, AUDIO, ARCHIVES, PROGRAMS, DOCUMENTS, CODE] {
            assert!(!extensions(f).is_empty());
            for e in extensions(f) {
                assert_eq!(family(e), f, "{e}");
            }
        }
        assert!(extensions(OTHER).is_empty());
        assert_eq!(family("xyz"), OTHER);
        assert_eq!(family("ts"), VIDEO, "the first family that lists it");
    }
}
