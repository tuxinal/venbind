fn main() {
    #[cfg(all(feature = "node", not(test)))]
    napi_build::setup();
}
