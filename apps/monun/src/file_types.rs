//! Windows: registers 모눈 for the spreadsheet files it opens, for the current user only (no
//! administrator rights). Explorer then lists 모눈 under Open with, Settings › Apps › Default apps
//! offers it, and files whose default app is 모눈 show its icon.
//!
//! Windows leaves the choice of default app to the user: once these entries exist, picking 모눈
//! with "Always" in Open with (or in Default apps) makes it the default for that type.
//!
//! The entries point at the running `monun.exe` and are written again on every normal start of
//! a release build, so a rebuilt or moved checkout keeps them right.

/// Where per-user file classes live under `HKEY_CURRENT_USER`.
const CLASSES: &str = r"Software\Classes";

/// The name Explorer shows for the app in Open with and Default apps.
const APP_NAME: &str = "모눈";

/// The app's capabilities, which Default apps reads through `RegisteredApplications`.
const CAPABILITIES: &str = r"Software\Monun\Capabilities";

/// The file types 모눈 opens (`FileKind` in the engine), each with the type name Explorer shows
/// for files whose default app is 모눈. Of the source files, those of everyday development that
/// aren't programs Windows runs (no .bat, .cmd or .ps1).
const TYPES: [(&str, &str); 44] = [
    (".xlsx", "모눈 워크시트"),
    (".xlsm", "모눈 매크로 사용 워크시트"),
    (".xltx", "모눈 서식 파일"),
    (".xltm", "모눈 매크로 사용 서식 파일"),
    (".xls", "모눈 97-2003 워크시트"),
    (".xlsb", "모눈 바이너리 워크시트"),
    (".ods", "모눈 OpenDocument 스프레드시트"),
    (".csv", "모눈 쉼표로 구분된 값 파일"),
    (".tsv", "모눈 탭으로 구분된 값 파일"),
    (".md", "모눈 Markdown 문서"),
    (".sql", "모눈 SQL 파일"),
    (".txt", "모눈 텍스트 문서"),
    (".pdf", "모눈 PDF 문서"),
    (".docx", "모눈 Word 문서"),
    (".doc", "모눈 Word 97-2003 문서"),
    (".rtf", "모눈 서식 있는 텍스트 문서"),
    (".odt", "모눈 OpenDocument 텍스트"),
    (".hwp", "모눈 한글 문서"),
    (".hwpx", "모눈 한글 문서 (HWPX)"),
    (".png", "모눈 PNG 그림"),
    (".jpg", "모눈 JPEG 그림"),
    (".jpeg", "모눈 JPEG 그림"),
    (".gif", "모눈 GIF 그림"),
    (".bmp", "모눈 BMP 그림"),
    (".webp", "모눈 WebP 그림"),
    (".html", "모눈 HTML 문서"),
    (".htm", "모눈 HTML 문서"),
    (".pptx", "모눈 PowerPoint 프레젠테이션"),
    (".java", "모눈 Java 소스 파일"),
    (".jsp", "모눈 JSP 파일"),
    (".js", "모눈 JavaScript 파일"),
    (".ts", "모눈 TypeScript 파일"),
    (".vue", "모눈 Vue 파일"),
    (".xml", "모눈 XML 문서"),
    (".yml", "모눈 YAML 파일"),
    (".yaml", "모눈 YAML 파일"),
    (".properties", "모눈 Properties 파일"),
    (".json", "모눈 JSON 파일"),
    (".css", "모눈 CSS 파일"),
    (".scss", "모눈 SCSS 파일"),
    (".gradle", "모눈 Gradle 파일"),
    (".kt", "모눈 Kotlin 파일"),
    (".py", "모눈 Python 파일"),
    (".log", "모눈 로그 파일"),
];

/// One string value under `HKEY_CURRENT_USER`: its key, its name (empty for the key's default
/// value) and its data.
#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub name: String,
    pub data: String,
}

fn entry(key: impl Into<String>, name: impl Into<String>, data: impl Into<String>) -> Entry {
    Entry { key: key.into(), name: name.into(), data: data.into() }
}

/// The class (ProgID) 모눈 registers for `ext`: `Monun.xlsx` for `.xlsx`.
fn prog_id(ext: &str) -> String {
    format!("Monun{ext}")
}

/// Every value that registers `exe` (the full path of `monun.exe`) for [`TYPES`]: a class per
/// type with 모눈's icon and open command, listed in the type's Open with; the app under
/// `Applications`; and its capabilities for Default apps.
pub fn entries(exe: &str) -> Vec<Entry> {
    let icon = format!("\"{exe}\",0");
    let open = format!("\"{exe}\" \"%1\"");
    let app = format!(r"{CLASSES}\Applications\monun.exe");
    let mut out = Vec::new();
    for (ext, type_name) in TYPES {
        let class = format!(r"{CLASSES}\{}", prog_id(ext));
        out.push(entry(&class, "", type_name));
        out.push(entry(format!(r"{class}\DefaultIcon"), "", &icon));
        out.push(entry(format!(r"{class}\shell\open\command"), "", &open));
        out.push(entry(format!(r"{CLASSES}\{ext}\OpenWithProgids"), prog_id(ext), ""));
        out.push(entry(format!(r"{app}\SupportedTypes"), ext, ""));
        out.push(entry(format!(r"{CAPABILITIES}\FileAssociations"), ext, prog_id(ext)));
    }
    out.push(entry(&app, "FriendlyAppName", APP_NAME));
    out.push(entry(format!(r"{app}\DefaultIcon"), "", &icon));
    out.push(entry(format!(r"{app}\shell\open\command"), "", &open));
    out.push(entry(CAPABILITIES, "ApplicationName", APP_NAME));
    out.push(entry(CAPABILITIES, "ApplicationDescription", "엑셀 파일을 여는 개인용 스프레드시트"));
    out.push(entry(CAPABILITIES, "ApplicationIcon", &icon));
    out.push(entry(r"Software\RegisteredApplications", "Monun", CAPABILITIES));
    out
}

/// Writes [`entries`] for `exe` under `HKEY_CURRENT_USER`.
#[cfg(target_os = "windows")]
pub fn register(exe: &str) -> windows_registry::Result<()> {
    for e in entries(exe) {
        windows_registry::CURRENT_USER.create(&e.key)?.set_string(&e.name, &e.data)?;
    }
    Ok(())
}

/// Registers the running `monun.exe`; a failure is logged, and the app starts all the same.
#[cfg(target_os = "windows")]
pub fn register_running_app() {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            log::warn!("file types: no path for monun.exe: {e}");
            return;
        }
    };
    let Some(exe) = exe.to_str() else {
        log::warn!("file types: {} is not valid Unicode", exe.display());
        return;
    };
    match register(exe) {
        Ok(()) => log::debug!("file types: registered {exe}"),
        Err(e) => log::warn!("file types: registering {exe}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Program Files\Monun\monun.exe";

    fn value<'a>(entries: &'a [Entry], key: &str, name: &str) -> Option<&'a str> {
        entries.iter().find(|e| e.key == key && e.name == name).map(|e| e.data.as_str())
    }

    #[test]
    fn each_type_opens_in_monun_with_its_icon() {
        let entries = entries(EXE);
        for (ext, type_name) in TYPES {
            let class = format!(r"Software\Classes\Monun{ext}");
            assert_eq!(value(&entries, &class, ""), Some(type_name));
            assert_eq!(value(&entries, &format!(r"{class}\DefaultIcon"), ""), Some(r#""C:\Program Files\Monun\monun.exe",0"#));
            // Quoted, so a path with spaces opens the file instead of running `C:\Program`.
            assert_eq!(value(&entries, &format!(r"{class}\shell\open\command"), ""), Some(r#""C:\Program Files\Monun\monun.exe" "%1""#));
        }
    }

    #[test]
    fn open_with_and_default_apps_list_every_type() {
        let entries = entries(EXE);
        for (ext, _) in TYPES {
            let prog_id = format!("Monun{ext}");
            assert_eq!(value(&entries, &format!(r"Software\Classes\{ext}\OpenWithProgids"), &prog_id), Some(""));
            assert_eq!(value(&entries, r"Software\Classes\Applications\monun.exe\SupportedTypes", ext), Some(""));
            assert_eq!(value(&entries, r"Software\Monun\Capabilities\FileAssociations", ext), Some(prog_id.as_str()));
        }
        assert_eq!(value(&entries, r"Software\Classes\Applications\monun.exe", "FriendlyAppName"), Some("모눈"));
        assert_eq!(value(&entries, r"Software\RegisteredApplications", "Monun"), Some(r"Software\Monun\Capabilities"));
    }

    #[test]
    fn every_type_registered_is_one_monun_opens() {
        for (ext, _) in TYPES {
            assert!(monun_engine::io::FileKind::from_path(&format!("a{ext}")).is_some(), "{ext}");
        }
    }

    #[test]
    fn nothing_claims_the_extensions_default() {
        // The extension keys only gain an Open with entry: which app is the default stays the
        // user's choice.
        let entries = entries(EXE);
        for (ext, _) in TYPES {
            assert!(!entries.iter().any(|e| e.key == format!(r"Software\Classes\{ext}") && e.name.is_empty()));
        }
    }
}
