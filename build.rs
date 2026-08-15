fn main() {
    println!("cargo:rerun-if-changed=assets/icons/meshpad_icon.ico");
    #[cfg(windows)]
    {
        winresource::WindowsResource::new()
            .set_icon("assets/icons/meshpad_icon.ico")
            .compile()
            .expect("embed Windows application icon");
    }
}
