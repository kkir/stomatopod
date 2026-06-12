fn main() {
    // Inputs that should trigger a rebuild when they change.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/tracker.js");
}
