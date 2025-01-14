use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let dst = cmake::Config::new("vendor")
        .define("USE_XINERAMA", "OFF")
        .define("USE_XTEST", "OFF")
        .define("USE_XT", "OFF")
        .define("CMAKE_INSTALL_LIBDIR", "lib")
        .build();
    println!("cargo:rustc-link-search=native={}/lib", dst.display());
    println!("cargo:rustc-link-lib=static=uiohook");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        println!("cargo:rustc-link-lib=user32");
    }
    if std::env::var_os("CARGO_CFG_UNIX").is_some() {
        println!("cargo:rustc-link-lib=X11");
        println!("cargo:rustc-link-lib=xcb");
        println!("cargo:rustc-link-lib=X11-xcb");
        println!("cargo:rustc-link-lib=xkbcommon-x11");
        println!("cargo:rustc-link-lib=xkbcommon");
        println!("cargo:rustc-link-lib=Xtst");
    }

    let bindings = bindgen::Builder::default()
        .header("vendor/include/uiohook.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate bindings");
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");

    if std::env::var_os("CARGO_CFG_UNIX").is_some() {
        let bindings_linux = bindgen::Builder::default()
            .header("vendor/src/x11/input_helper.h")
            .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
            .generate()
            .expect("Unable to generate bindings");
        bindings_linux
            .write_to_file(out_path.join("linux_helper_bindings.rs"))
            .expect("Couldn't write bindings!");
    }
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let bindings_windows = bindgen::Builder::default()
            .header("stdint.h")
            .header("vendor/src/windows/input_helper.h")
            .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
            .generate()
            .expect("Unable to generate bindings");
        bindings_windows
            .write_to_file(out_path.join("windows_helper_bindings.rs"))
            .expect("Couldn't write bindings!");
    }
}
