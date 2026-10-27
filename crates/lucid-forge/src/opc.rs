//! The Open Packaging Conventions parts of a package: `[Content_Types].xml` and `_rels/.rels`.
//!
//! Both parts are optional for Lucid, but real packages carry them and Microsoft Help Viewer
//! expects them, so `build` writes them unless `--no-opc` is given.

/// The MIME type written into `[Content_Types].xml` for a part extension.
///
/// Unknown extensions map to `application/octet-stream`; extension comparison is
/// case-insensitive.
pub fn mime_for_extension(extension: &str) -> &'static str {
    match extension.to_ascii_lowercase().as_str() {
        "htm" | "html" => "text/html",
        "xhtml" => "application/xhtml+xml",
        "css" => "text/css",
        "js" | "mjs" => "application/javascript",
        "json" => "application/json",
        "xml" => "application/xml",
        "rels" => "application/vnd.openxmlformats-package.relationships+xml",
        "txt" | "log" | "md" => "text/plain",
        "gif" => "image/gif",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "eot" => "application/vnd.ms-fontobject",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// The content type of the `HelpContentSet` manifest part.
pub const MANIFEST_CONTENT_TYPE: &str = "application/vnd.ms-help.contentset+xml";

/// Build `[Content_Types].xml`.
///
/// `extensions` are the extensions present in the package (lowercase, no dots, deduplicated);
/// they are written as sorted `<Default>` entries so the output is stable. When `manifest` is
/// given, an `<Override>` for that part is added — that is how real packages declare the
/// `*.msha` part.
pub fn content_types_xml(extensions: &[String], manifest: Option<&str>) -> String {
    let mut extensions: Vec<String> = extensions
        .iter()
        .map(|extension| extension.trim_start_matches('.').to_ascii_lowercase())
        .filter(|extension| !extension.is_empty())
        .collect();
    extensions.sort();
    extensions.dedup();

    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n",
    );
    for extension in &extensions {
        out.push_str(&format!(
            "  <Default Extension=\"{}\" ContentType=\"{}\" />\n",
            crate::manifest::escape_xml(extension),
            mime_for_extension(extension)
        ));
    }
    if let Some(manifest) = manifest {
        out.push_str(&format!(
            "  <Override PartName=\"/{}\" ContentType=\"{}\" />\n",
            crate::manifest::escape_xml(manifest.trim_start_matches('/')),
            MANIFEST_CONTENT_TYPE
        ));
    }
    out.push_str("</Types>\n");
    out
}

/// Build `_rels/.rels` pointing at the manifest part (the MSHC relationship type).
pub fn package_rels_xml(manifest: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n\
         \x20 <Relationship Id=\"rId1\" Type=\"http://schemas.microsoft.com/help/2010/06/msha\" \
         Target=\"{}\" />\n\
         </Relationships>\n",
        crate::manifest::escape_xml(manifest.trim_start_matches('/'))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_types_are_sorted_and_deduplicated() {
        let xml = content_types_xml(
            &["bmp".to_owned(), "CSS".to_owned(), "css".to_owned()],
            Some("lucid-help.msha"),
        );
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<Types"));
        let bmp = xml.find("Extension=\"bmp\"").unwrap();
        let css = xml.find("Extension=\"css\"").unwrap();
        assert!(bmp < css);
        assert_eq!(xml.matches("Extension=\"css\"").count(), 1);
        assert!(xml.contains("<Default Extension=\"css\" ContentType=\"text/css\" />"));
        assert!(xml.contains("<Override PartName=\"/lucid-help.msha\""));
    }

    #[test]
    fn rels_point_at_the_manifest() {
        let xml = package_rels_xml("demo.msha");
        assert!(xml.contains("Type=\"http://schemas.microsoft.com/help/2010/06/msha\""));
        assert!(xml.contains("Target=\"demo.msha\""));
        assert!(xml.contains("Id=\"rId1\""));
    }

    #[test]
    fn mime_guessing_knows_the_documented_extensions() {
        assert_eq!(mime_for_extension("HTM"), "text/html");
        assert_eq!(mime_for_extension("css"), "text/css");
        assert_eq!(mime_for_extension("js"), "application/javascript");
        assert_eq!(mime_for_extension("bmp"), "image/bmp");
        assert_eq!(mime_for_extension("unknown"), "application/octet-stream");
    }
}
