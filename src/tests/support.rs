// shared helpers for tests that compose layers from synthetic rootfs trees,
// so no network fetch is needed. a "real" binary inside the layer is the
// host's static-ish /bin/true, copied in, which is enough to exec

use nexus::{Core, Layout, Store};
use std::path::{Path, PathBuf};

// write a tiny rootfs at `src`: an executable (/bin/true copied from host)
// plus a marker file unique to the layer, so a process inside can prove it
// is rooted in the composed tree. fails if the essential pieces can't be
// written, so a broken setup surfaces instead of producing an empty tree
pub fn synthetic_rootfs(src: &Path, marker: &str) -> Result<(), String> {
    let mk = |p: std::io::Result<()>, what: &str| p.map_err(|e| format!("{what}: {e}"));
    mk(std::fs::create_dir_all(src.join("bin")), "mkdir bin")?;
    mk(std::fs::create_dir_all(src.join("usr/bin")), "mkdir usr/bin")?;
    mk(std::fs::write(src.join(marker), b"layer"), "write marker")?;
    // a working executable to run inside the layer
    std::fs::copy("/bin/true", src.join("usr/bin/true")).map_err(|e| format!("copy true: {e}"))?;
    // convenience copies some tests exec; absence is not fatal
    let _ = std::fs::copy("/usr/bin/test", src.join("usr/bin/test"));
    let _ = std::fs::copy("/bin/true", src.join("bin/true"));
    Ok(())
}

// import `src` and check it out as layer `id`'s tree where the overlay
// backend reads its lower. returns the store so the caller can verify it
pub fn import_layer(layout: &Layout, id: &str, src: &Path) -> Result<Store, String> {
    let store = Store::new(layout.store());
    let tree = store.import(src).map_err(|e| format!("import: {e}"))?;
    let lower = layout.store().join(id);
    store.checkout(&tree, &lower).map_err(|e| format!("checkout: {e}"))?;
    Ok(store)
}

// write a layers.toml with one or more layer blocks and a nexus.toml naming
// the backend. each layer is (id, type, priority, libc-name, extra-lines)
pub struct LayerSpec {
    pub id: &'static str,
    pub class: &'static str,
    pub priority: u32,
    pub libc: &'static str,
    pub loader: Option<&'static str>,
    pub extra: &'static str,
}

pub fn write_state(layout: &Layout, backend: &str, specs: &[LayerSpec]) -> std::io::Result<()> {
    let dir = layout.state();
    std::fs::create_dir_all(&dir)?;
    let mut toml = String::new();
    for s in specs {
        toml.push_str(&format!(
            "[layer.{}]\ntype = \"{}\"\npriority = {}\n{}",
            s.id, s.class, s.priority, s.extra
        ));
        toml.push_str(&format!("[layer.{}.libc]\nname = \"{}\"\n", s.id, s.libc));
        if let Some(l) = s.loader {
            toml.push_str(&format!("loader = \"{l}\"\n"));
        }
    }
    std::fs::write(dir.join("layers.toml"), toml)?;
    // keep the cgroup subtree inside the sandbox so a run touches nothing on
    // the host cgroup tree; teardown of the temp dir removes it
    let cgroup_root = layout.root().join(".cgroup");
    std::fs::write(
        dir.join("nexus.toml"),
        format!("backend = \"{backend}\"\ncgroup_root = \"{}\"\n", cgroup_root.display()),
    )?;
    Ok(())
}

// the first real file entry in a directory, skipping bookkeeping dotfiles
pub fn first_object(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.file_name().map(|n| !n.to_string_lossy().starts_with('.')).unwrap_or(false))
}

// write a minimal layers.toml + nexus.toml for a single shadowed layer
// used by the Shadow harness and the rootless tests  - same format, same
// cgroup isolation
pub fn write_shadow_state(
    layout: &Layout,
    layer: &str,
    libc_name: &str,
    loader: &str,
) -> std::io::Result<()> {
    let dir = layout.state();
    std::fs::create_dir_all(&dir)?;
    let layers = format!(
        "[layer.{layer}]\n\
         type = \"shadowed\"\n\
         priority = 1\n\
         [layer.{layer}.libc]\n\
         name = \"{libc_name}\"\n\
         loader = \"{loader}\"\n"
    );
    std::fs::write(dir.join("layers.toml"), layers)?;
    let cgroup_root = layout.root().join(".cgroup");
    std::fs::write(
        dir.join("nexus.toml"),
        format!("backend = \"overlay\"\ncgroup_root = \"{}\"\n", cgroup_root.display()),
    )?;
    Ok(())
}

// build a Core from on-disk state written by write_state. a parse error is
// returned, not swallowed into an empty default that would make a later step
// fail for the wrong reason
pub fn open_core(layout: &Layout) -> Result<Core, String> {
    let layers = nexus::load_layers(&layout.state().join("layers.toml"))
        .map_err(|e| format!("load layers: {e}"))?;
    let system = nexus::load_system(&layout.state().join("nexus.toml"))
        .map_err(|e| format!("load system: {e}"))?;
    Ok(Core::open(layout.clone(), layers, system))
}
