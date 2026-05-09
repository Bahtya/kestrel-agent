fn main() {
    prost_build::Config::new()
        .compile_protos(&["proto/pbbp2.proto"], &["proto/"])
        .expect("Failed to compile pbbp2.proto");
}
