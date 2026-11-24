//! Windows only: put the icon and the version info into repo-status.exe. Needs the resource
//! compiler of a Windows build (rc.exe with MSVC); a cross check from Linux skips it.

fn main() {
    println!("cargo:rerun-if-changed=data/slashrepo.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") || !cfg!(windows) {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("data/slashrepo.ico").set("FileDescription", "Repo Status").set("ProductName", "slashRepo");
    if let Err(e) = res.compile() {
        // a missing resource compiler must not fail the build: the exe then has the default icon
        println!("cargo:warning=no icon in the exe: {e}");
    }
}
