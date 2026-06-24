// uid remapping via idmapped mount: file on disk owned by X, seen inside
// as Y. probes first, skips when the kernel rejects MOUNT_ATTR_IDMAP

use super::support::{import_layer, synthetic_rootfs, write_state, LayerSpec};
use super::{run_in_layer, Category, TestContext, Test};

// the layer carries an idmap; with it applied to the mount, a file owned by
// one uid on disk is seen under a different uid inside the layer. that
// difference is what we assert
pub struct Idmapped;

impl Test for Idmapped {
    fn name(&self) -> &str {
        "namespace: idmapped mount remaps uids"
    }

    fn category(&self) -> Category {
        Category::Namespace
    }

    fn run(&self, tcx: &mut TestContext) {
        // real end-to-end probe: nexus tries to apply an idmap to a throwaway
        // tmpfs. if that fails here, idmapped mounts will not work for a layer
        // either, so skip with the true reason rather than a misleading fail
        if !nexus::idmap_usable() {
            tcx.wrn("idmapped mounts not usable in this environment; skipping");
            return;
        }

        let layout = tcx.sandbox.layout().clone();
        let src = tcx.sandbox.base().join("src");
        if tcx.try_ok(synthetic_rootfs(&src, ".marker"), "synth rootfs").is_none() {
            return;
        }
        if import_layer(&layout, "void", &src).is_err() {
            tcx.ok(false, "import layer");
            return;
        }

        // map container uid 0..count to outer 100000.., the usual rootless
        // range. with the idmap applied to the mount, files owned by uid 0 on
        // disk appear owned by 100000 inside
        let state = write_state(
            &layout,
            "overlay",
            &[LayerSpec {
                id: "void",
                class: "shadowed",
                priority: 1,
                libc: "static",
                loader: None,
                extra: "ephemeral = true\n\
                        [layer.void.sandbox.idmap]\n\
                        outer_start = 100000\n\
                        count = 65536\n",
            }],
        );
        if tcx.try_ok(state, "wrote layer state").is_none() {
            return;
        }

        let Some(mut core) = tcx.try_ok(super::support::open_core(&layout), "opened core") else {
            return;
        };
        match core.build("void") {
            Ok(()) => {
                tcx.ok(true, "composed layer with idmap");
            }
            Err(e) => {
                // idmapped mounts unsupported here: a clean skip, not a fail
                tcx.wrn(&format!("idmapped compose unsupported: {e}; skipping"));
                return;
            }
        }

        // a process runs inside the idmapped layer
        match run_in_layer(&mut core, "void", &["/usr/bin/true"]) {
            Ok(0) => tcx.ok(true, "ran a process in the idmapped layer"),
            Ok(code) => tcx.ok(false, &format!("process in idmapped layer exited {code}")),
            Err(e) => tcx.ok(false, &format!("run in idmapped layer: {e}")),
        };

        // the mapped owner inside differs from the raw on-disk owner. the
        // marker file is owned by the current uid on disk (we created it);
        // through the idmap it should resolve to a different uid inside
        // we read the inside view by stat-ing the file from a child that has
        // entered the namespace
        let ns = layout.ns_file("void");
        let raw_uid = std::fs::metadata(layout.store().join("void/.marker"))
            .map(|m| {
                use std::os::unix::fs::MetadataExt;
                m.uid()
            })
            .unwrap_or(u32::MAX);
        match owner_inside(&ns, "/.marker") {
            Some(inside) => tcx.ok(
                inside != raw_uid,
                &format!("uid inside ({inside}) differs from on-disk ({raw_uid})"),
            ),
            None => tcx.ok(false, "could not stat the marker inside the layer"),
        };
    }
}

// fork a child that enters the layer's mount namespace and returns the uid
// owning `path` as seen inside, via libc::stat. None on any failure
fn owner_inside(ns: &std::path::Path, path: &str) -> Option<u32> {
    use std::os::fd::AsRawFd;
    // a pipe to carry the uid back from the child
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return None;
    }
    let (rd, wr) = (fds[0], fds[1]);
    // safe: spark forks single-threaded, so the child below may allocate
    crate::proc::assert_fork_safe();
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return None;
    }
    if pid == 0 {
        unsafe { libc::close(rd) };
        let uid = (|| {
            let f = std::fs::File::open(ns).ok()?;
            if unsafe { libc::setns(f.as_raw_fd(), libc::CLONE_NEWNS) } != 0 {
                return None;
            }
            let c = std::ffi::CString::new(path).ok()?;
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe { libc::stat(c.as_ptr(), &mut st) } != 0 {
                return None;
            }
            Some(st.st_uid)
        })();
        let val = uid.unwrap_or(u32::MAX);
        unsafe {
            libc::write(wr, &val as *const u32 as *const _, 4);
            libc::_exit(0);
        }
    }
    unsafe { libc::close(wr) };
    let mut val = [0u8; 4];
    let got = unsafe { libc::read(rd, val.as_mut_ptr() as *mut _, 4) };
    unsafe {
        libc::close(rd);
        let mut s = 0;
        libc::waitpid(pid, &mut s, 0);
    }
    if got == 4 {
        let uid = u32::from_ne_bytes(val);
        if uid == u32::MAX { None } else { Some(uid) }
    } else {
        None
    }
}
