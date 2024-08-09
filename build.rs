fn main() {
    #[cfg(all(feature = "node", not(test)))]
    node_bindgen::build::configure();
}