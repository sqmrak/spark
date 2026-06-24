// the erofs backend: probe for kernel support, verify warm fails cleanly
// when the kernel lacks erofs or no image exists

use super::support::{import_layer, synthetic_rootfs, write_state, LayerSpec};
use super::{Category, TestContext, Test};

pub struct Erofs;

impl Test for Erofs {
    fn name(&self) -> &str {
        "namespace: erofs probe"
    }

    fn category(&self) -> Category {
        Category::Namespace
    }

    fn run(&self, tcx: &mut TestContext) {
        let layout = tcx.sandbox.layout().clone();
        let src = tcx.sandbox.base().join("src");
        if tcx.try_ok(synthetic_rootfs(&src, ".marker"), "synth rootfs").is_none() {
            return;
        }
        if import_layer(&layout, "void", &src).is_err() {
            tcx.ok(false, "import layer");
            return;
        }
        let state = write_state(
            &layout,
            "erofs",
            &[LayerSpec {
                id: "void",
                class: "shadowed",
                priority: 1,
                libc: "static",
                loader: None,
                extra: "",
            }],
        );
        if tcx.try_ok(state, "wrote layer state").is_none() {
            return;
        }
        let Some(mut core) = tcx.try_ok(super::support::open_core(&layout), "opened core") else {
            return;
        };

        if !erofs_supported() {
            tcx.wrn("erofs not supported on this kernel; skipping");
            tcx.ok(core.build("void").is_err(), "erofs build fails cleanly without kernel support");
            return;
        }

        tcx.ok(core.build("void").is_err(), "erofs build without an image reports an error");
    }
}

fn erofs_supported() -> bool {
    std::fs::read_to_string("/proc/filesystems")
        .map(|s| s.lines().any(|l| l.split_whitespace().last() == Some("erofs")))
        .unwrap_or(false)
}
