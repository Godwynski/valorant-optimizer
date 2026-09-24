use std::path::PathBuf;

fn main() {
    slint_build::compile("ui/appwindow.slint").expect("Failed to compile Slint UI definitions");

    #[cfg(windows)]
    {
        let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        // Link against workspace native lib directory
        let workspace_lib_dir = manifest_dir.parent().unwrap().parent().unwrap().join("target").join("lib");
        if workspace_lib_dir.exists() {
            println!("cargo:rustc-link-search=native={}", workspace_lib_dir.display());
        }
    }
}
