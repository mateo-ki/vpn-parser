fn main() {
    println!("cargo:rerun-if-changed=assets/app-icon.ico");
    println!("cargo:rerun-if-changed=assets/windows.rc");

    #[cfg(windows)]
    embed_resource::compile("assets/windows.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("compile Windows application resources");
}
