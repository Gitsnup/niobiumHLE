//! Windows Phone `.xap` Silverlight application packages.
//!
//! A `.xap` is a ZIP archive whose payload is *managed IL*: the game
//! ships as .NET assemblies (`<entry>.dll` plus satellites) with XNA
//! content compiled to `.xnb` resources, and `WMAppManifest.xml` /
//! `AppManifest.xaml` describe the package. PocketHLE runs managed
//! images through a host runtime (see `pocket-cli/src/managed.rs`),
//! so the loader's job here is only to identify the package and pick
//! the entry-point assembly.
//!
//! Parsing is deliberately attribute-scanning rather than a real XML
//! parser: both manifests are machine-written single-line documents
//! with a stable attribute set, and a scanner keeps the crate free of
//! an XML dependency for two well-known files.

/// What the two manifests say about a Windows Phone package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XapInfo {
    /// Display title, from `WMAppManifest.xml`'s `App/@Title`.
    pub title: String,
    /// Publisher or author, from `App/@Publisher` then `App/@Author`.
    pub publisher: Option<String>,
    /// Target platform, from `Deployment/@AppPlatformVersion`
    /// (e.g. `7.1` for Windows Phone Mango).
    pub app_platform_version: Option<String>,
    /// Entry-point assembly name, from `AppManifest.xaml`'s
    /// `Deployment/@EntryPointAssembly`. The launchable image is
    /// `<entry_assembly>.dll` next to the manifest.
    pub entry_assembly: String,
}

/// Parse `WMAppManifest.xml` and `AppManifest.xaml` contents.
///
/// Returns `None` when either document is missing the attribute the
/// loader cannot do without — the entry-point assembly — which is how
/// a random ZIP with the two files but no real package structure is
/// rejected rather than imported as a broken game.
pub fn parse(manifest_xml: &str, app_manifest_xaml: &str) -> Option<XapInfo> {
    let entry_assembly = xaml_attribute(app_manifest_xaml, "EntryPointAssembly")?;
    let title = xap_app_attribute(manifest_xml, "Title").unwrap_or_default();
    Some(XapInfo {
        title,
        publisher: xap_app_attribute(manifest_xml, "Publisher")
            .or_else(|| xap_app_attribute(manifest_xml, "Author")),
        app_platform_version: xaml_attribute(manifest_xml, "AppPlatformVersion"),
        entry_assembly,
    })
}

/// Read `name="value"` for the first element that carries the
/// attribute, scanning attributes element-agnostically.
fn xaml_attribute(document: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=");
    let start = document.find(&needle)?;
    let after = &document[start + needle.len()..];
    for quote in ['"', '\''] {
        if let Some(rest) = after.strip_prefix(quote) {
            let end = rest.find(quote)?;
            return Some(unescape_xml(&rest[..end]));
        }
    }
    None
}

/// Read `name="value"` from `WMAppManifest.xml`'s `<App>` element
/// specifically. The document also carries `<Title>` inside
/// `<Tokens>` (the live-tile label), which frequently differs from the
/// application title, so a document-wide scan would answer the wrong
/// question.
fn xap_app_attribute(document: &str, name: &str) -> Option<String> {
    let app = document.find("<App ")?;
    let end = document[app..].find('>')? + app;
    xaml_attribute(&document[app..end], name)
}

fn unescape_xml(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}
