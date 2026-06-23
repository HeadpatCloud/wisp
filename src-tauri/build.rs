fn main() {
    // Use new_without_app_manifest so tauri-build's resource.lib only carries
    // VERSIONINFO + ICON (no manifest). We then add the manifest separately via
    // compile_for_everything so it reaches ALL targets including lib unit tests.
    // This avoids CVT1100 (duplicate resource) that would occur if the manifest
    // appeared in both tauri-build's rustc-link-arg-bins AND our rustc-link-arg.
    tauri_build::try_build(
        tauri_build::Attributes::new().windows_attributes(
            tauri_build::WindowsAttributes::new_without_app_manifest(),
        ),
    )
    .expect("failed to run tauri build script");

    #[cfg(target_os = "windows")]
    {
        let out = std::env::var("OUT_DIR").unwrap();
        let rc_path = std::path::Path::new(&out).join("manifest.rc");
        // comctl32 v6 manifest - needed by tao's TaskDialogIndirect import.
        // rustc-link-arg-bins on tauri's resource.lib no longer includes this
        // (new_without_app_manifest), so we embed it here for every target.
        std::fs::write(
            &rc_path,
            r#"1 24
{
"<assembly xmlns=""urn:schemas-microsoft-com:asm.v1"" manifestVersion=""1.0"">"
"  <dependency>"
"    <dependentAssembly>"
"      <assemblyIdentity"
"        type=""win32"""
"        name=""Microsoft.Windows.Common-Controls"""
"        version=""6.0.0.0"""
"        processorArchitecture=""*"""
"        publicKeyToken=""6595b64144ccf1df"""
"        language=""*"""
"      />"
"    </dependentAssembly>"
"  </dependency>"
"</assembly>"
}
"#,
        )
        .expect("failed to write manifest.rc");
        embed_resource::compile_for_everything(rc_path, embed_resource::NONE)
            .manifest_optional()
            .expect("failed to embed manifest");
    }
}
