
fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "CheckpointPro");
        res.set(
            "FileDescription",
            "CheckpointPro — save, track, and restore your project's progress",
        );
        res.set("CompanyName", "CheckpointPro");
        res.set("LegalCopyright", "© 2026 ");
        res.set("FileVersion", "0.1.0.0");
        res.set("ProductVersion", "0.1.0.0");
        res.set("OriginalFilename", "checkpoint.exe");
        res.compile().unwrap();
    }
}
