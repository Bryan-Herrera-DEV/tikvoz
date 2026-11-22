//! Incrusta metadatos de versión y un manifest en el .exe de Windows.
//! Un ejecutable sin metadatos es justo el perfil que la heurística de
//! Windows Defender (`!ml`) marca como sospechoso; con esto se ve como una app normal.

fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set("ProductName", "TikVoz")
            .set(
                "FileDescription",
                "TikVoz - lector de chat de TikTok y Kick",
            )
            .set("CompanyName", "TikVoz")
            .set("LegalCopyright", "MIT")
            .set("OriginalFilename", "tikvoz.exe")
            .set("InternalName", "tikvoz")
            .set_manifest(&manifest());
        if let Err(e) = res.compile() {
            // Sin rc.exe (Windows SDK) igual compilamos, solo sin metadatos.
            println!("cargo:warning=no se pudieron incrustar recursos de Windows: {e}");
        }
    }
}

/// La versión del manifest sale de Cargo.toml (formato a.b.c.0 que exige Windows).
#[cfg(windows)]
fn manifest() -> String {
    let version = format!(
        "{}.0",
        env!("CARGO_PKG_VERSION")
            .split('-')
            .next()
            .unwrap_or("0.0.0")
    );
    MANIFEST.replace("{VERSION}", &version)
}

#[cfg(windows)]
const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity version="{VERSION}" processorArchitecture="amd64" name="TikVoz" type="win32"/>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security><requestedPrivileges>
      <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
    </requestedPrivileges></security>
  </trustInfo>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application><supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/></application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <activeCodePage xmlns="http://schemas.microsoft.com/SMI/2019/WindowsSettings">UTF-8</activeCodePage>
    </windowsSettings>
  </application>
</assembly>
"#;
