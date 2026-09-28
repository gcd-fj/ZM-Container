use std::{env, path::Path};

fn main() {
    println!("cargo:rerun-if-env-changed=ZM_WINDOWS_RESOURCE");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && let Some(resource) = env::var_os("ZM_WINDOWS_RESOURCE")
    {
        let resource = Path::new(&resource);
        assert!(
            resource.is_absolute() && resource.is_file(),
            "invalid Windows resource path"
        );
        println!("cargo:rerun-if-changed={}", resource.display());
        println!("cargo:rustc-link-arg-bin=zm-linux={}", resource.display());
    }
}
