use std::path::PathBuf;

fn main() {
    // Inputs that should trigger a rebuild when they change.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/marketing.css");
    println!("cargo:rerun-if-changed=../../assets/dist/marketing.css");
    println!("cargo:rerun-if-changed=../../assets/vendor/anime.min.js");
    println!("cargo:rerun-if-changed=../../tailwind.config.js");
    println!("cargo:rerun-if-changed=../../crates/web/templates/marketing");

    // The compiled marketing CSS is embedded via include_str!() at compile
    // time. If it isn't on disk, fail loud with a clear remedy.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dist_css = manifest_dir.join("../../assets/dist/marketing.css");
    if !dist_css.exists() {
        println!(
            "cargo:warning=Missing assets/dist/marketing.css. Run `npm install && npm run build:assets` from the workspace root before `cargo build`."
        );
        panic!("missing compiled marketing CSS at {}", dist_css.display());
    }

    let anime_js = manifest_dir.join("../../assets/vendor/anime.min.js");
    if !anime_js.exists() {
        println!(
            "cargo:warning=Missing assets/vendor/anime.min.js. Run `npm install && npm run vendor:anime` from the workspace root."
        );
        panic!("missing vendored anime.min.js at {}", anime_js.display());
    }
}
