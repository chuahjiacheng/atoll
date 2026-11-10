fn main() {
    tauri_build::build();
    // Tauri's menus (muda) can import TaskDialogIndirect, which exists only in
    // Common Controls 6. The app gets version 6 from the manifest tauri-build
    // embeds; test programs have no manifest, and depending on how a debug
    // build happens to split its code, Windows then refused to start them
    // (STATUS_ENTRYPOINT_NOT_FOUND). Loading comctl32 on first use instead of
    // at start-up fixes that: tests never call it, and the app still gets 6.
    #[cfg(windows)]
    {
        println!("cargo:rustc-link-arg=/DELAYLOAD:comctl32.dll");
        println!("cargo:rustc-link-lib=dylib=delayimp");
    }
}
