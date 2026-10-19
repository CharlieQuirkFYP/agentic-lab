fn main() {
    if let Ok(directory) = std::env::var("DEP_LITERT_LIB_DIR") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{directory}");
    }
}
